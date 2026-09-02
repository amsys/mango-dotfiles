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

Every lock path — `SUPER,l` and hypridle's own 300s idle-timeout listener —
calls `sleep-lock.py lock` instead of bare `swaylock`, so the pomodoro pauses
for every lock, not only across suspend (a phase boundary landing while the
screen was locked used to spawn a full-screen rofi overlay behind the lock
surface, which then came back deaf on unlock — see `mango-bard`'s pomo.rs
`alert()` and `focus-break.sh`'s own swaylock guard, the second line of
defense). `lock` checks for an already-running swaylock first, the same
guard `begin_sleep()` uses below, and does nothing if it finds one — the
already-running lock's own caller owns that pause/unpause pair.

If a lock is already running when the machine is asked to *sleep* (a
manual `lock` invocation, or hypridle's idle listener, won by a hair before
`PrepareForSleep`), `begin_sleep()` below trusts it and releases the
inhibitor at once rather than waiting again — a second `swaylock --ready-fd`
would just exit 2 ("another lockscreen running").

Run as a systemd user unit (../../systemd/mango-sleep-lock.service) — it
must never die silently, since once `before_sleep_cmd` is gone it is the
only thing locking the screen before sleep.

    sleep-lock.py         run the daemon (systemd ExecStart)
    sleep-lock.py lock     lock the screen and pause the pomodoro — what
                            every lock keybind/listener should call instead
                            of bare `swaylock`
    sleep-lock.py test     non-disruptive self-check: no real lock spawned
    SIGUSR1                rehearse a real lock+pause+unlock cycle without
                            waiting for an actual suspend (see selftest()'s
                            own comment for why `test` alone can't do this)
"""

import os
import shutil
import signal
import subprocess
import sys
import tempfile
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


# Module-level, not SleepLock methods: neither touches `self` — both just
# shell out to `mango-bard` — so `do_lock()` (the `lock` subcommand) can
# reuse them without standing up a dbus bus and a SleepLock instance it has
# no other use for.
def pause_pomodoro():
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


def unpause_pomodoro():
    try:
        subprocess.run(["mango-bard", "pomo", "unpause"], capture_output=True, timeout=2)
    except (OSError, subprocess.TimeoutExpired) as e:
        log(f"mango-bard pomo unpause failed: {e}")


def do_lock():
    """`sleep-lock.py lock` — the one command every lock path (`SUPER,l` and
    hypridle's 300s idle listener) calls instead of bare `swaylock`. Pauses
    the pomodoro before swaylock draws and resumes it once swaylock exits,
    using the same "ok" vs "noop" ownership contract as `begin_sleep()`
    below, so a pause already held by a suspend in progress is never
    double-unpaused.

    Checks for an already-running swaylock first, same guard and same
    reasoning as `begin_sleep()`'s own check: swaylock refuses a second
    instance (exit 2), and pausing here first would leave this call owning
    a pause it has no way to time an unpause for, since `swaylock` would
    return the moment the second instance is rejected, not when the real
    lock clears.
    """
    if running_swaylock_pid() is not None:
        log("lock: another swaylock is already running — nothing to do")
        return 0
    owns_pause = pause_pomodoro()
    ret = subprocess.run(["swaylock"]).returncode
    if owns_pause:
        unpause_pomodoro()
        log("lock: pomodoro resumed")
    return ret


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
        self.proc = None  # our own swaylock child, if we spawned it — see begin_sleep()

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

    # --------------------------------------------------------------- locking

    def begin_sleep(self, rehearsal=False):
        self.rehearsal = rehearsal
        self.owns_pause = pause_pomodoro()
        self._t0 = time.monotonic()

        pid = running_swaylock_pid()
        if pid is not None:
            log(f"swaylock already running (pid {pid}) — trusting it, releasing at once")
            self._watch_pid_for_unlock(pid)
            self._done_locking()
            return

        r, w = os.pipe()
        try:
            proc = subprocess.Popen(["swaylock", "--ready-fd", str(w)], pass_fds=(w,))
        finally:
            os.close(w)
        # Keep the handle so _on_pid_exit can wait() it once the pidfd fires —
        # a pidfd watch alone does not reap; an un-waited child stays a zombie
        # for the life of this daemon, and pgrep (unlike pidof) matches zombies.
        self.proc = proc
        self.ready_fd = r
        # Same watch as the "already running" branch above — without it,
        # only that branch ever fires _on_pid_exit, so owns_pause is never
        # cleared and the pomodoro stays paused forever after a suspend that
        # spawned its own swaylock (SUPER+SHIFT+L, power-key hold,
        # battery-guard's emergency suspend).
        self._watch_pid_for_unlock(proc.pid)

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
            self._done_locking()
        else:
            # swaylock exited before writing the ready byte. The documented
            # case is benign (exit 2, "another lockscreen running" — a lock
            # is already on screen), but a crash or bad config looks the
            # same here and must not release the inhibitor early: hold until
            # _on_ready_timeout's bound instead of suspending unlocked.
            log(
                f"swaylock exited without confirming a lock ({elapsed_ms:.0f} ms in)"
                " — holding until the ready timeout"
            )
            self.io_id = None  # the watch already removed itself (one-shot)
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
        if self.proc is not None:
            self.proc.wait()  # pidfd already fired, so this cannot block
            self.proc = None
        log("swaylock has exited — treating as unlocked")
        if self.owns_pause:
            unpause_pomodoro()
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
    # against a throwaway child instead of a real swaylock. Routed through the
    # real handler (not a standalone callback) so this also proves
    # _on_pid_exit reaps its child — an unreaped pidfd-watched child is
    # exactly how a stray zombie swaylock broke SUPER+L's `pgrep` guard.
    loop = GLib.MainLoop()
    fired = {}
    proc = subprocess.Popen(["sh", "-c", "sleep 0.2"])
    handler.proc = proc

    def on_exit(channel, condition, pfd):
        fired["ok"] = True
        handler._on_pid_exit(channel, condition, pfd)
        loop.quit()
        return False

    pfd = os.pidfd_open(proc.pid, 0)
    ch = GLib.IOChannel.unix_new(pfd)
    GLib.io_add_watch(ch, GLib.PRIORITY_DEFAULT, GLib.IO_IN, on_exit, pfd)
    GLib.timeout_add(3000, lambda: (loop.quit(), False)[1])
    loop.run()
    assert fired.get("ok"), "pidfd exit watch never fired"
    assert handler.proc is None, "_on_pid_exit did not reap the child"
    assert proc.returncode is not None, "child left as a zombie"

    selftest_lock()
    print("ok")


def selftest_lock():
    """`do_lock()`'s call order — pause, then swaylock, then unpause — using
    stub `mango-bard` and `swaylock` on PATH. Never touches the real logind
    inhibitor or the real pomodoro."""
    tmp = tempfile.mkdtemp()
    try:
        calls = os.path.join(tmp, "calls")

        def write_stub(name, body):
            path = os.path.join(tmp, name)
            with open(path, "w") as f:
                f.write(f"#!/usr/bin/env bash\n{body}\n")
            os.chmod(path, 0o755)

        write_stub(
            "mango-bard",
            f'if [ "$2" = pause ]; then echo -n ok; fi; echo "$2" >>"{calls}"',
        )
        write_stub("swaylock", f'echo lock >>"{calls}"')
        write_stub("pidof", "exit 1")  # no swaylock running yet

        env = dict(os.environ, PATH=f"{tmp}:{os.environ['PATH']}")
        ret = subprocess.run(
            [sys.executable, __file__, "lock"], env=env, capture_output=True, text=True
        )
        assert ret.returncode == 0, f"lock exited {ret.returncode}: {ret.stderr}"
        with open(calls) as f:
            order = f.read().split()
        assert order == ["pause", "lock", "unpause"], f"wrong call order: {order}"

        # A "noop" pause (another lock already owns it) must not unpause.
        os.remove(calls)
        write_stub("mango-bard", f'echo "$2" >>"{calls}"')  # pause -> stdout "" -> noop
        ret = subprocess.run(
            [sys.executable, __file__, "lock"], env=env, capture_output=True, text=True
        )
        assert ret.returncode == 0, f"lock exited {ret.returncode}: {ret.stderr}"
        with open(calls) as f:
            order = f.read().split()
        assert order == ["pause", "lock"], f"noop pause was unpaused: {order}"

        # An already-running swaylock must skip pause/lock/unpause entirely.
        os.remove(calls)
        write_stub("pidof", "echo 12345")
        ret = subprocess.run(
            [sys.executable, __file__, "lock"], env=env, capture_output=True, text=True
        )
        assert ret.returncode == 0, f"lock exited {ret.returncode}: {ret.stderr}"
        assert not os.path.exists(calls), "an already-running lock should be left alone"
    finally:
        shutil.rmtree(tmp)


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
    if len(sys.argv) > 1 and sys.argv[1] == "lock":
        sys.exit(do_lock())
    main()
