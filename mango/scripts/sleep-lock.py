#!/usr/bin/env python3
"""Lock the screen before the machine sleeps, and pause the pomodoro across it.

hypr/hypridle.conf used to own this via `before_sleep_cmd` + `inhibit_sleep
= 3`. That cannot work on mango: `inhibit_sleep = 3` only waits for a real
lock if the compositor advertises `hyprland-lock-notify-v1`; mango exposes
`ext-session-lock-v1` (wlroots) instead, and hypridle silently falls back to
a plain inhibitor that it releases the instant `before_sleep_cmd` is
*spawned* — not once swaylock has actually locked anything (confirmed
against hypridle v0.1.8 source, `Hypridle.cpp`'s `handleDbusSleep` /
`uninhibitSleep`). No hypridle config value fixes that here.

This script takes over that half of hypridle's job with its own logind
delay inhibitor, held until `swaylock --ready-fd` confirms
`ext_session_lock_v1.locked` — a real, protocol-level "the screen is locked
now" signal (see swaylock's `main.c`: that fd write sits between the
`while (!state.locked)` dispatch loop and anything else). It also pauses
the pomodoro (`mango-bard pomo pause`) before sleep and resumes it
(`pomo unpause`) once the screen is unlocked again, so a suspend never
leaves the `CLOCK_REALTIME` phase timer to fire a stack of missed-boundary
alerts on wake (see pomo.rs's `catch_up`).

hypridle's own idle-timeout lock (the 300s listener in hypridle.conf) is
untouched and keeps calling plain `swaylock` on its own. If that lock is
already running when the machine is asked to sleep, this script trusts it
and releases the inhibitor at once rather than waiting again — a second
`swaylock --ready-fd` would just exit 2 ("another lockscreen running").
ponytail: this accepts a narrow, hard-to-hit race (idle-lock firing in the
same instant as a manual suspend, before it has actually drawn the lock
screen) rather than unifying every lock path through one owner; upgrade to
a single-owner `lock` subcommand here if that race is ever observed.

Run as a systemd user unit (../../systemd/mango-sleep-lock.service) — it
must never die silently, since once `before_sleep_cmd` is gone it is the
only thing locking the screen before sleep.

    sleep-lock.py         run the daemon (systemd ExecStart)
    sleep-lock.py test     non-disruptive self-check: no real lock spawned
    SIGUSR1                rehearse a real lock+pause+unlock cycle without
                            waiting for an actual suspend (see selftest()'s
                            own comment for why `test` alone can't do this)
"""

import os
import signal
import subprocess
import sys
import time

import gi

gi.require_version("Gio", "2.0")
gi.require_version("GLib", "2.0")
gi.require_version("GLibUnix", "2.0")
from gi.repository import Gio, GLib, GLibUnix  # noqa: E402

BUS_NAME = "org.freedesktop.login1"
BUS_PATH = "/org/freedesktop/login1"
BUS_IFACE = "org.freedesktop.login1.Manager"

# Bounded so a wedged swaylock can never hold up a real suspend past
# logind's own backstop (`InhibitDelayMaxSec`, 5s by default on this
# machine — `busctl get-property org.freedesktop.login1
# /org/freedesktop/login1 org.freedesktop.login1.Manager
# InhibitDelayMaxUSec`). Comfortably under it, not equal to it: this
# should win the race and log a clean warning, not get cut off by logind.
LOCK_READY_TIMEOUT_SEC = 3.0


def log(msg):
    print(f"mango-sleep-lock: {msg}", file=sys.stderr, flush=True)


def running_swaylock_pid():
    out = subprocess.run(["pidof", "swaylock"], capture_output=True, text=True).stdout.split()
    return int(out[0]) if out else None


class SleepLock:
    def __init__(self, bus):
        self.bus = bus
        self.inhibit_fd = None
        self.owns_pause = False  # did *we* pause the pomodoro this cycle?
        self.rehearsal = False  # SIGUSR1 test run — see begin_sleep()
        self.ready_fd = None
        self.timeout_id = None
        self.io_id = None
        self._t0 = 0.0

    # ------------------------------------------------------- logind inhibitor

    def acquire(self):
        # PrepareForSleep(true) firing twice without a (false) between is
        # not something logind's own contract allows, but never double
        # acquire regardless of why it happened.
        if self.inhibit_fd is not None:
            return
        self.inhibit_fd = self._inhibit()
        log(f"inhibitor acquired (fd {self.inhibit_fd})")

    def _inhibit(self):
        # A helper, not inlined: `values`/`fd_list` must fall out of scope
        # here so their own (separately dup'd) copy of the fd is dropped —
        # otherwise closing only the fd this returns leaves the inhibitor
        # held forever, since logind only releases it once *every* dup of
        # the underlying pipe write-end is closed. Verified live: closing
        # our fd alone, with `fd_list` still referenced by the caller, left
        # the inhibitor listed in `systemd-inhibit --list` after close().
        values, fd_list = self.bus.call_with_unix_fd_list_sync(
            BUS_NAME,
            BUS_PATH,
            BUS_IFACE,
            "Inhibit",
            GLib.Variant(
                "(ssss)",
                (
                    "sleep",
                    "mango-sleep-lock",
                    "lock the screen and pause the pomodoro before sleep",
                    "delay",
                ),
            ),
            GLib.VariantType.new("(h)"),
            Gio.DBusCallFlags.NONE,
            -1,
            None,
        )
        return fd_list.get(values.unpack()[0])

    def release(self):
        if self.inhibit_fd is None:
            return
        os.close(self.inhibit_fd)
        self.inhibit_fd = None
        log("inhibitor released")

    # ------------------------------------------------------------- pomodoro

    def pause_pomodoro(self):
        try:
            out = subprocess.run(
                ["mango-bard", "pomo", "pause"],
                capture_output=True,
                text=True,
                timeout=2,
            ).stdout.strip()
        except (OSError, subprocess.TimeoutExpired) as e:
            log(f"mango-bard pomo pause failed, leaving the timer as-is: {e}")
            return False
        # "ok" means it paused a *running* block — that, and only that, is
        # ours to undo on unlock. "noop" (idle, or already paused by hand)
        # must not be resumed by us later.
        return out == "ok"

    def unpause_pomodoro(self):
        try:
            subprocess.run(
                ["mango-bard", "pomo", "unpause"], capture_output=True, timeout=2
            )
        except (OSError, subprocess.TimeoutExpired) as e:
            log(f"mango-bard pomo unpause failed: {e}")

    # --------------------------------------------------------------- locking

    def begin_sleep(self, rehearsal=False):
        self.rehearsal = rehearsal
        self.owns_pause = self.pause_pomodoro()
        self._t0 = time.monotonic()

        pid = running_swaylock_pid()
        if pid is not None:
            log(f"swaylock already running (pid {pid}) — trusting it, releasing at once")
            self._watch_pid_for_unlock(pid)
            self._done_locking()
            return

        r, w = os.pipe()
        try:
            subprocess.Popen(["swaylock", "--ready-fd", str(w)], pass_fds=(w,))
        finally:
            os.close(w)
        self.ready_fd = r

        ch = GLib.IOChannel.unix_new(r)
        self.io_id = GLib.io_add_watch(
            ch, GLib.PRIORITY_DEFAULT, GLib.IO_IN | GLib.IO_HUP, self._on_ready
        )
        self.timeout_id = GLib.timeout_add(
            int(LOCK_READY_TIMEOUT_SEC * 1000), self._on_ready_timeout
        )

    def _on_ready(self, channel, condition):
        data = os.read(self.ready_fd, 1)
        elapsed_ms = (time.monotonic() - self._t0) * 1000
        if data:
            log(f"locked in {elapsed_ms:.0f} ms")
        else:
            # swaylock exited before writing the ready byte — most likely
            # exit 2, "another lockscreen running", a race against the
            # idle-timeout lock this script's own docstring accepts.
            log(f"swaylock exited without confirming a lock ({elapsed_ms:.0f} ms in)")
        self._done_locking()
        return False  # one-shot watch

    def _on_ready_timeout(self):
        log(f"lock did not confirm within {LOCK_READY_TIMEOUT_SEC:.0f}s — releasing anyway")
        self._done_locking()
        return False

    def _done_locking(self):
        if self.timeout_id is not None:
            GLib.source_remove(self.timeout_id)
            self.timeout_id = None
        if self.io_id is not None:
            GLib.source_remove(self.io_id)
            self.io_id = None
        if self.ready_fd is not None:
            os.close(self.ready_fd)
            self.ready_fd = None
        self.release()
        if self.rehearsal:
            # SIGUSR1 has no matching real wake to re-arm on — restore the
            # standing invariant ("the daemon always holds an inhibitor
            # except mid-sleep") right away instead of leaving the machine
            # unprotected until the next real suspend/resume cycle.
            self.acquire()
            self.rehearsal = False

    def _watch_pid_for_unlock(self, pid):
        # Works whether or not this pid is our own child: a pidfd becomes
        # readable on process exit regardless of parent relationship
        # (verified live — a grandchild reparented away from this process
        # still woke a poll() on its pidfd the moment it exited).
        pfd = os.pidfd_open(pid, 0)
        ch = GLib.IOChannel.unix_new(pfd)
        GLib.io_add_watch(ch, GLib.PRIORITY_DEFAULT, GLib.IO_IN, self._on_pid_exit, pfd)

    def _on_pid_exit(self, channel, condition, pfd):
        os.close(pfd)
        log("swaylock has exited — treating as unlocked")
        if self.owns_pause:
            self.unpause_pomodoro()
            log("pomodoro resumed")
        self.owns_pause = False
        return False  # one-shot watch

    # ------------------------------------------------------------ dbus glue

    def on_prepare_for_sleep(self, connection, sender, path, iface, signal_name, params):
        (going_to_sleep,) = params.unpack()
        if going_to_sleep:
            log("PrepareForSleep(true)")
            self.begin_sleep()
        else:
            log("PrepareForSleep(false) — re-arming for the next sleep")
            self.acquire()

    def on_sigusr1(self):
        log("SIGUSR1 — rehearsing a real lock+pause cycle (no suspend involved)")
        self.begin_sleep(rehearsal=True)
        return True  # keep the signal source


def selftest():
    """Non-disruptive: never spawns a real swaylock, so it is always safe
    to run — including as part of routine verification. It cannot exercise
    the actual lock; that needs a live SIGUSR1 rehearsal or a real suspend
    (see the module docstring)."""
    for tool in ("swaylock", "mango-bard", "pidof"):
        assert (
            subprocess.run(["which", tool], capture_output=True).returncode == 0
        ), f"{tool} not on PATH"

    # Count-based, not a bare "not in" check — a real mango-sleep-lock.service
    # instance may legitimately already be running and holding its own
    # inhibitor while this test runs; that must not look like a leak.
    def inhibitor_count():
        out = subprocess.run(
            ["systemd-inhibit", "--list"], capture_output=True, text=True
        ).stdout
        return out.count("mango-sleep-lock")

    before = inhibitor_count()
    bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
    handler = SleepLock(bus)
    handler.acquire()
    assert handler.inhibit_fd is not None
    assert inhibitor_count() == before + 1, "acquire() did not add an inhibitor"
    handler.release()
    assert handler.inhibit_fd is None
    assert inhibitor_count() == before, "inhibitor still held after release()"

    # The same pidfd + GLib.IOChannel plumbing `_watch_pid_for_unlock` uses,
    # against a throwaway child instead of a real swaylock.
    loop = GLib.MainLoop()
    fired = {}
    proc = subprocess.Popen(["sh", "-c", "sleep 0.2"])

    def on_exit(channel, condition, pfd):
        fired["ok"] = True
        os.close(pfd)
        loop.quit()
        return False

    pfd = os.pidfd_open(proc.pid, 0)
    ch = GLib.IOChannel.unix_new(pfd)
    GLib.io_add_watch(ch, GLib.PRIORITY_DEFAULT, GLib.IO_IN, on_exit, pfd)
    GLib.timeout_add(3000, lambda: (loop.quit(), False)[1])
    loop.run()
    assert fired.get("ok"), "pidfd exit watch never fired"

    print("ok")


def main():
    bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
    handler = SleepLock(bus)
    handler.acquire()
    bus.signal_subscribe(
        BUS_NAME,
        BUS_IFACE,
        "PrepareForSleep",
        BUS_PATH,
        None,
        Gio.DBusSignalFlags.NONE,
        handler.on_prepare_for_sleep,
    )
    GLibUnix.signal_add(GLib.PRIORITY_DEFAULT, signal.SIGUSR1, handler.on_sigusr1)
    GLib.MainLoop().run()


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "test":
        selftest()
        sys.exit(0)
    main()
