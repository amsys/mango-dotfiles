#!/usr/bin/env python3
"""Tiered power-button handling, replacing logind's single-action HandlePowerKey.

    tap  (< 1.0s)      -> powermenu.sh, the session menu
    hold (1.0 - 3.5s)  -> systemctl suspend, on release
    hold (~4s)         -> the firmware cuts power. Not ours, not avoidable.

Why a watcher instead of a compositor bind or logind: mango has no release
binds, so a key bind cannot measure a hold at all, and logind offers exactly one
short-press action plus a long-press whose threshold is hardcoded at 5s — past
the ~4s ACPI power-button override this machine implements in firmware, so it
would never fire. Reading evdev directly is the only place the duration exists.

Nothing is scheduled anywhere near the 4s override, so a clean shutdown can
never be cut off half way. Shutdown lives in the power menu instead.

logind must stand down or it will suspend the moment the key goes down:

    /etc/systemd/logind.conf.d/10-power.conf
        HandlePowerKey=ignore
        HandlePowerKeyLongPress=ignore

    powerkey.py         exec-once from mango/config.conf — watch the key
    powerkey.py test    assert the tier thresholds, no device needed
"""

import os
import signal
import struct
import subprocess
import sys
import time
import glob

# struct input_event { struct timeval time; __u16 type, code; __s32 value; }
# timeval is two longs, so this is 24 bytes on 64-bit and 16 on 32-bit.
EVENT_FMT = "llHHi"
EVENT_SIZE = struct.calcsize(EVENT_FMT)

EV_KEY = 0x01
KEY_POWER = 116

TAP_MAX = 1.0     # below this, the menu
SUSPEND_MAX = 3.5  # above this, assume they are reaching for the firmware override

# Sibling script, not wlogout directly — it warns about anything mid-flight
# before putting a Shutdown button under the cursor, and owns the menu geometry.
# dirname(__file__) deliberately does not resolve the symlink: this file is
# reached as ~/.config/mango/scripts/powerkey.py, which is where its sibling is.
TAP_CMD = [os.path.join(os.path.dirname(os.path.abspath(__file__)), "powermenu.sh")]
SUSPEND_CMD = ["systemctl", "suspend"]


def action(held):
    """Seconds held -> the command to run, or None."""
    if held < TAP_MAX:
        return TAP_CMD
    if held <= SUSPEND_MAX:
        return SUSPEND_CMD
    return None


def find_device():
    """Path of the evdev node named 'Power Button'.

    Matched by name rather than pinned to event0: the numbering is discovery
    order and moves when a USB keyboard is plugged in at boot.
    """
    for name_path in sorted(glob.glob("/sys/class/input/event*/device/name")):
        try:
            with open(name_path) as fh:
                if fh.read().strip() == "Power Button":
                    node = name_path.split("/")[4]  # .../input/eventN/device/name
                    return "/dev/input/" + node
        except OSError:
            continue
    return None


def watch(path):
    # start_new_session below calls setsid, which does NOT reparent: wlogout
    # stays our child and goes defunct the moment it exits, because this loop
    # never waits on anything. One zombie per power-key tap, for the life of the
    # session. SIG_IGN on SIGCHLD hands the reaping to the kernel, which is the
    # whole answer for a process that fires and forgets — it would break
    # Popen.wait/poll, and nothing here calls either.
    signal.signal(signal.SIGCHLD, signal.SIG_IGN)

    down = None
    # Unbuffered: buffered reads would hand back a batch of events long after
    # the press, and every timestamp here comes from the kernel anyway.
    with open(path, "rb", buffering=0) as dev:
        while True:
            data = dev.read(EVENT_SIZE)
            if not data or len(data) < EVENT_SIZE:
                return
            sec, usec, etype, code, value = struct.unpack(EVENT_FMT, data)
            if etype != EV_KEY or code != KEY_POWER:
                continue
            now = sec + usec / 1e6
            if value == 1:      # press
                down = now
            elif value == 0 and down is not None:   # release
                cmd = action(now - down)
                down = None
                if cmd:
                    # Detached: this process must go straight back to reading,
                    # and wlogout outlives the keypress by definition.
                    subprocess.Popen(cmd, start_new_session=True,
                                     stdout=subprocess.DEVNULL,
                                     stderr=subprocess.DEVNULL)
            # value == 2 is autorepeat while held; the release carries the total.


def selftest():
    assert action(0.05) == TAP_CMD
    assert action(0.99) == TAP_CMD
    assert action(1.0) == SUSPEND_CMD
    assert action(2.5) == SUSPEND_CMD
    assert action(3.5) == SUSPEND_CMD
    # Past this they are holding for the firmware override — doing anything
    # would mean racing a power cut with a suspend.
    assert action(3.6) is None
    assert action(10) is None
    # The device must be findable on this machine, or the watcher is a no-op.
    assert find_device(), "no evdev node named 'Power Button'"
    assert struct.calcsize(EVENT_FMT) in (16, 24)

    # A detached child must not go defunct — this leaked one wlogout per tap.
    # Spawned the same way watch() spawns wlogout, so the check fails if that
    # call ever stops being fire-and-forget.
    signal.signal(signal.SIGCHLD, signal.SIG_IGN)
    child = subprocess.Popen(["true"], start_new_session=True,
                             stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL)
    for _ in range(50):
        time.sleep(0.02)
        try:
            state = open(f"/proc/{child.pid}/stat").read().split(") ", 1)[1][0]
        except OSError:
            break   # reaped, the entry is gone
        if state != "Z":
            break
    else:
        raise AssertionError(f"pid {child.pid} still defunct: SIGCHLD not ignored")
    signal.signal(signal.SIGCHLD, signal.SIG_DFL)
    print("ok")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "test":
        selftest()
        sys.exit(0)
    dev = find_device()
    if not dev:
        sys.exit("powerkey: no 'Power Button' evdev node")
    if not os.access(dev, os.R_OK):
        sys.exit(f"powerkey: cannot read {dev} — is this user in the 'input' group?")
    watch(dev)
