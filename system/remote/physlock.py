#!/usr/bin/env python3
"""Root input-grab helper for a physical lock during a remote session
(SPEC.md docs/features/bar-status-panel/SPEC.md §7.2-7.3).

While a VNC client is connected, this grabs (EVIOCGRAB) every physical
keyboard, touchpad, mouse and touchscreen, so a person at the laptop
cannot type or click into the session mango is also serving to the
remote user. It never grabs the power button, the lid switch, or any
device under /sys/devices/virtual/input (uinput — kdeconnectd's virtual
devices). wayvnc does not use evdev at all: it uses the Wayland
virtual-keyboard/pointer protocol. Bluetooth LE devices (BlueZ uhid,
under /sys/devices/virtual/misc/uhid) are physical and are grabbed. See
"Why a lock, not just a blank" in README.md.

Root is required: the session user is deliberately not a member of
`input` (see system/vault/ for the same policy applied to the power
button alone), so grabbing every input device needs its own daemon,
started and stopped by the user session over sudo (see
mango-physlock.sudoers) — never enabled at login.

Esc held >= HOLD_THRESHOLD_SEC on any grabbed keyboard is a takeover
request: the person at the laptop wants the session back. This helper
holds the grab throughout, so only it ever sees that key — mango and
the remote user do not. It hands the lock/restore sequence to the user
session (remote.sh --takeover) and only releases the grab once that
command confirms success; a failed or slow takeover leaves the grab in
place and lets a second Esc hold retry (README.md's "Why a lock, not
just a blank" has the end-to-end ordering and its residual gaps).

    physlock.py            run the daemon (systemd ExecStart, root)
    physlock.py --selftest  non-disruptive self-check: no /dev/input
                             access, no real runuser/remote.sh call
    physlock.py --on-stop   ExecStopPost target: lock the active
                             session via runuser, so a crash or a clean
                             stop never ends unlocked (fail closed)
    SIGTERM                 release every grab and exit
"""

import fcntl
import glob
import os
import pwd
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import time

# ---------------------------------------------------------------- ioctl plumbing
# Linux's _IOC() macro (asm-generic/ioctl.h), reproduced here because fcntl.ioctl
# needs the numeric request code and this is stdlib-only (no python-evdev). Self-
# checked against the well-known EVIOCGRAB value: _IOC(1,'E',0x90,4) == 0x40044590.
_IOC_NRBITS = 8
_IOC_TYPEBITS = 8
_IOC_SIZEBITS = 14
_IOC_DIRBITS = 2

_IOC_NRSHIFT = 0
_IOC_TYPESHIFT = _IOC_NRSHIFT + _IOC_NRBITS
_IOC_SIZESHIFT = _IOC_TYPESHIFT + _IOC_TYPEBITS
_IOC_DIRSHIFT = _IOC_SIZESHIFT + _IOC_SIZEBITS

_IOC_WRITE = 1
_IOC_READ = 2


def _IOC(direction, type_char, nr, size):
    return (
        (direction << _IOC_DIRSHIFT)
        | (ord(type_char) << _IOC_TYPESHIFT)
        | (nr << _IOC_NRSHIFT)
        | (size << _IOC_SIZESHIFT)
    )


def EVIOCGBIT(ev, length):
    return _IOC(_IOC_READ, "E", 0x20 + ev, length)


def EVIOCGPROP(length):
    return _IOC(_IOC_READ, "E", 0x09, length)


def EVIOCGNAME(length):
    return _IOC(_IOC_READ, "E", 0x06, length)


EVIOCGRAB = _IOC(_IOC_WRITE, "E", 0x90, struct.calcsize("i"))

# input-event-codes.h subset — only what classification and the Esc watch need.
EV_KEY = 0x01
EV_REL = 0x02
EV_ABS = 0x03
EV_SW = 0x05

KEY_ESC = 1
KEY_A = 30
KEY_POWER = 116
BTN_LEFT = 0x110
REL_X = 0x00
REL_Y = 0x01
ABS_MT_SLOT = 0x2F
SW_LID = 0x00
INPUT_PROP_DIRECT = 1

# struct input_event { struct timeval time; __u16 type, code; __s32 value; } —
# same layout and the same reasoning (24 bytes on 64-bit) as powerkey.py's
# EVENT_FMT, which this file deliberately mirrors rather than re-deriving.
EVENT_FMT = "llHHi"
EVENT_SIZE = struct.calcsize(EVENT_FMT)

# Esc held this long on a grabbed keyboard is a takeover request (SPEC.md §7.3).
HOLD_THRESHOLD_SEC = 2.0

# Bounded so a wedged remote.sh --takeover can never hold this process (and the
# grab it is deciding whether to release) forever. Generous next to the ~6s
# sleep-lock.py `lock --until-locked` budget that --takeover's own first step
# spends (HYPRLOCK_LOCKED_TIMEOUT_SEC + SWAYLOCK_READY_TIMEOUT_SEC in
# sleep-lock.py), leaving room for the stop-wayvnc/--restore/destroy-output
# steps after it.
TAKEOVER_TIMEOUT_SEC = 15.0

# New devices are picked up within this long even if the udev watch below
# never fires (wrong kernel assumption, blocked netlink, anything) — see
# open_uevent_socket(). "udev add event or every 2s", per SPEC.md §7's open item
# about a USB keyboard plugged in mid-session.
POLL_INTERVAL_SEC = 2.0


def log(msg):
    print(f"mango-physlock: {msg}", file=sys.stderr, flush=True)


# -------------------------------------------------------------- device capture

def get_bits(fd, ev, nbytes=96):
    """EVIOCGBIT's capability bitmask for one event type. Never raises: a
    device that stops answering mid-rescan (unplugged) just classifies as
    "no capabilities", which classify_bits() turns into "skip it", not a
    crash that would take the whole rescan down with it."""
    buf = bytearray(nbytes)
    try:
        fcntl.ioctl(fd, EVIOCGBIT(ev, nbytes), buf)
    except OSError:
        pass
    return bytes(buf)


def get_props(fd, nbytes=8):
    buf = bytearray(nbytes)
    try:
        fcntl.ioctl(fd, EVIOCGPROP(nbytes), buf)
    except OSError:
        pass
    return bytes(buf)


def get_name(fd, length=256):
    buf = bytearray(length)
    try:
        fcntl.ioctl(fd, EVIOCGNAME(length), buf)
    except OSError:
        return ""
    return buf.split(b"\x00", 1)[0].decode(errors="replace")


def test_bit(bits, n):
    idx = n // 8
    if bits is None or idx >= len(bits):
        return False
    return bool(bits[idx] & (1 << (n % 8)))


def classify_bits(name, sys_path, key_bits, rel_bits, abs_bits, sw_bits, prop_bits):
    """Pure classifier — no fd, no ioctl — so --selftest covers the one
    place a wrong exclusion (grabbing the power button, the lid switch, or
    a uinput device) would otherwise be invisible until it happened on
    real hardware. Returns "keyboard" / "mouse" / "touchpad" /
    "touchscreen", or None to skip the device.

    Exclusions are checked first and win over any capability match:

    - /sys/devices/virtual/input/* — uinput. kdeconnectd's virtual
      keyboard/mouse live here, and wayvnc drives the session through the
      Wayland virtual-keyboard protocol, not evdev at all — so remote
      input never needs an exclusion of its own, but a *local* uinput
      device (rare, but possible) must not be grabbed either: it is not
      "a keyboard a person at the laptop can reach with their hands".
    - "Power Button" / "Lid Switch" by name (same names powerkey.py and
      the ACPI drivers use), and defensively: a device whose only
      meaningful EV_KEY bit is KEY_POWER, or any EV_SW device advertising
      SW_LID, even if some other machine's firmware names them
      differently.
    """
    name = (name or "").strip()
    sys_path = sys_path or ""
    # Only /devices/virtual/input/ (uinput). Not all of /devices/virtual/:
    # BlueZ puts every Bluetooth LE keyboard and mouse under
    # /sys/devices/virtual/misc/uhid/, and those are physical devices that
    # a person at the laptop can use.
    if "/devices/virtual/input/" in sys_path:
        return None
    if name in ("Power Button", "Lid Switch"):
        return None
    if test_bit(sw_bits, SW_LID):
        return None
    if test_bit(key_bits, KEY_POWER) and not test_bit(key_bits, KEY_A):
        return None
    if test_bit(abs_bits, ABS_MT_SLOT):
        return "touchscreen" if test_bit(prop_bits, INPUT_PROP_DIRECT) else "touchpad"
    if test_bit(rel_bits, REL_X) and test_bit(rel_bits, REL_Y) and test_bit(key_bits, BTN_LEFT):
        return "mouse"
    if test_bit(key_bits, KEY_A):
        return "keyboard"
    return None


def probe_device(path):
    """Open `path` long enough to read its name, sysfs origin and
    capability bitmasks, then close it — the real grab (a separate open)
    happens only once classify_bits() says to keep it. Returns None for
    anything that cannot be opened or queried; a device node racing a
    rescan (just unplugged) must never raise out of this."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
    except OSError:
        return None
    try:
        sys_path = os.path.realpath(f"/sys/class/input/{os.path.basename(path)}/device")
        return {
            "path": path,
            "name": get_name(fd),
            "sys_path": sys_path,
            "key_bits": get_bits(fd, EV_KEY),
            "rel_bits": get_bits(fd, EV_REL),
            "abs_bits": get_bits(fd, EV_ABS),
            "sw_bits": get_bits(fd, EV_SW),
            "prop_bits": get_props(fd),
        }
    finally:
        os.close(fd)


def prune_disconnected(grabbed, trackers, path_exists=os.path.exists, closer=os.close):
    """Drop every `grabbed` entry whose `/dev/input/eventN` path no longer
    exists, closing its fd first. Pure enough to --selftest with a fake
    `grabbed`/`trackers` and a fake `path_exists` — no real /dev/input.

    Without this, an unplugged device's entry lingers in `grabbed` forever
    (nothing else ever removes one), and `rescan()`'s `if path in grabbed:
    continue` then wrongly skips that same path once it is reused — Linux
    reissues a freed input-event minor number to the next device that
    registers, and the common case that does this fast is the same
    keyboard: unplug and replug it (or let a Bluetooth one drop and
    reconnect) and it is very likely to come back as the exact same
    `/dev/input/eventN` while this daemon runs. Without pruning, that
    replugged keyboard — a genuinely new device, grabbed or not purely by
    coincidence of its path — is never probed again, let alone grabbed:
    the one failure mode that matters most here, since it is the person's
    own keyboard that silently stops being covered."""
    removed = []
    for path in list(grabbed.keys()):
        if path_exists(path):
            continue
        info = grabbed.pop(path)
        trackers.pop(info["fd"], None)
        try:
            closer(info["fd"])
        except OSError:
            pass
        removed.append(path)
    return removed


def grab(fd):
    fcntl.ioctl(fd, EVIOCGRAB, struct.pack("i", 1))


def ungrab(fd):
    fcntl.ioctl(fd, EVIOCGRAB, struct.pack("i", 0))


# --------------------------------------------------------------- hold detector

class EscHoldTracker:
    """Tracks one grabbed keyboard's Esc key. on_key() records press/
    release transitions (value 1/2 -> pressed, value 0 -> released, see
    run_daemon()); check_timeout() is what actually fires — called on
    every event AND on every poll-loop wakeup, so a continuous hold is
    still caught between repeat events (a keyboard's first autorepeat
    interval can exceed HOLD_THRESHOLD_SEC)."""

    def __init__(self, threshold=HOLD_THRESHOLD_SEC):
        self.threshold = threshold
        self._down_since = None
        self._fired = False

    def on_key(self, pressed, now):
        if pressed:
            if self._down_since is None:
                self._down_since = now
                self._fired = False
        else:
            self._down_since = None
            self._fired = False

    def check_timeout(self, now):
        """True exactly once per continuous hold, the moment `now` first
        crosses the threshold since the key went down. False for a key
        that is up, that has not reached the threshold yet, or that
        already fired for this hold (so a held key re-checked every poll
        does not re-trigger a takeover attempt every 2s thereafter)."""
        if self._down_since is None or self._fired:
            return False
        if now - self._down_since >= self.threshold:
            self._fired = True
            return True
        return False


class TakeoverController:
    """Runs the takeover command and decides whether the grab comes off.
    `run_cmd(timeout)` and `release_grab()` are injected so --selftest can
    exercise the success/failure/timeout ordering with fakes — no
    /dev/input, no real runuser or remote.sh call."""

    def __init__(self, run_cmd, release_grab, timeout=TAKEOVER_TIMEOUT_SEC):
        self.run_cmd = run_cmd
        self.release_grab = release_grab
        self.timeout = timeout

    def attempt(self):
        """Returns True (and has already released the grab) on success.
        Returns False (grab untouched) on a non-zero exit or a timeout —
        either way the caller is free to retry on the next qualifying
        Esc hold, since nothing here remembers a failed attempt."""
        try:
            rc = self.run_cmd(self.timeout)
        except subprocess.TimeoutExpired:
            log(f"takeover command did not finish within {self.timeout:.0f}s — keeping the grab")
            return False
        if rc == 0:
            self.release_grab()
            return True
        log(f"takeover command exited {rc} — keeping the grab")
        return False


def user_session_cmd(user, runtime_dir, wayland_display, argv):
    """Run `argv` as `user` inside that user's systemd manager, as a
    transient user service (`systemd-run --user --wait --pipe`).

    Why not plain `runuser ... argv`: the process then stays in this
    system unit's cgroup (/system.slice/mango-physlock.service).
    `sleep-lock.py lock --until-locked` must move its locker into a user
    scope, so a unit stop cannot kill the lock screen. The user manager
    can only move a process whose cgroup it owns; the common ancestor of
    /system.slice and the user tree is the root cgroup, which only root
    can write. The move failed, and no locker started (final review,
    2026-10-03). Inside a transient user service the move works.

    `--wait` returns the exit code of `argv` (checked: `sh -c 'exit 3'`
    gives 3), so the takeover keeps its "release only on 0" rule.
    A transient service does not inherit this environment, so `-E`
    passes the two variables the user side needs."""
    return [
        "runuser", "-u", user, "--",
        "env", f"XDG_RUNTIME_DIR={runtime_dir}",
        "systemd-run", "--user", "--wait", "--pipe", "--collect", "--quiet",
        "-E", f"XDG_RUNTIME_DIR={runtime_dir}",
        "-E", f"WAYLAND_DISPLAY={wayland_display}",
        "--", *argv,
    ]


def build_takeover_cmd(user, runtime_dir, wayland_display, home):
    """The exact command line SPEC.md §7.3 specifies: `runuser -u <user>
    -- env XDG_RUNTIME_DIR=... WAYLAND_DISPLAY=... <repo>/ironbar/scripts/
    remote.sh --takeover`. "<repo>" there means wherever the dotfiles
    repo's files are reachable from that user's session — which, same as
    every other systemd unit in this project (see mango-sleep-lock.service's
    `%h/.config/...` ExecStart), is `~/.config/ironbar/scripts/remote.sh`:
    install-config.sh symlinks the repo's ironbar/scripts tree straight
    into ~/.config, so this is the same file either way."""
    remote_sh = os.path.join(home, ".config", "ironbar", "scripts", "remote.sh")
    return user_session_cmd(user, runtime_dir, wayland_display, [remote_sh, "--takeover"])


def discover_session():
    """Find the active graphical session: the user, XDG_RUNTIME_DIR and
    WAYLAND_DISPLAY to run a user-side command as. A root system service
    carries none of this in its own environment, so it probes
    /run/user/<uid> for a live wayland-N socket — the same kind of probe
    fprint-notify.sh (the user D-Bus session) and rescue-outputs.sh (the
    mango IPC socket) already use, for the same reason: PID 1 hands a
    root service nothing about which session is on the seat.

    Returns (user, runtime_dir, wayland_display) or None. Not exercised by
    --selftest — it reads real system state, not something a fake can
    stand in for without a live session anyway."""
    for run_dir in sorted(glob.glob("/run/user/*")):
        uid_str = os.path.basename(run_dir)
        if not uid_str.isdigit():
            continue
        for sock_path in sorted(glob.glob(os.path.join(run_dir, "wayland-*"))):
            name = os.path.basename(sock_path)
            if name.endswith(".lock"):
                continue
            try:
                if not stat.S_ISSOCK(os.stat(sock_path).st_mode):
                    continue
            except OSError:
                continue
            try:
                user = pwd.getpwuid(int(uid_str)).pw_name
            except (KeyError, ValueError):
                continue
            return user, run_dir, name
    return None


# -------------------------------------------------------------------- udev watch

def open_uevent_socket():
    """Best-effort: a kernel uevent socket, so that run_daemon() wakes at
    once on a new input device and does not wait for the next
    POLL_INTERVAL_SEC poll. NETLINK_KOBJECT_UEVENT (15), multicast group
    1, is the kernel broadcast that udevd itself reads. Root has all the
    privilege that this needs.

    The socket goes into run_daemon()'s own select() set. (An earlier
    version read it in a thread that set a flag which the main loop
    never read, so a new device waited for the poll all the same.)

    Returns None if the socket cannot be opened. The 2 s poll then still
    finds a new device: a failure here delays a grab, it never skips one."""
    NETLINK_KOBJECT_UEVENT = 15
    try:
        sock = socket.socket(socket.AF_NETLINK, socket.SOCK_DGRAM, NETLINK_KOBJECT_UEVENT)
        sock.bind((0, 1))
        sock.setblocking(False)
    except OSError as e:
        log(f"udev netlink watch unavailable ({e}) — relying on the {POLL_INTERVAL_SEC:.0f}s poll only")
        return None
    return sock


def drain_uevents(sock):
    """Read every queued uevent. True if one of them adds an input device."""
    added = False
    while True:
        try:
            data = sock.recv(16384)
        except (BlockingIOError, InterruptedError):
            return added
        except OSError:
            # ENOBUFS (the queue overflowed) means events were lost.
            # Rescan to be safe.
            return True
        fields = data.split(b"\x00")
        if b"ACTION=add" in fields and b"SUBSYSTEM=input" in fields:
            added = True


# ------------------------------------------------------------ keyboard service

def service_keyboards(trackers, ready, now, read_events, on_fire, is_running):
    """Feed the Esc events of each ready keyboard to its tracker, and call
    on_fire() when a hold reaches the threshold.

    on_fire() can release every grab (a successful takeover clears
    `trackers`) and stop the daemon. So this checks is_running() and
    looks up each tracker again before each keyboard: with two or more
    keyboards (the laptop keyboard plus a Bluetooth one), a takeover
    from the first keyboard must not make the next pass raise KeyError.
    Such an exception ends the daemon with a non-zero code after a
    takeover that succeeded.

    read_events(fd) gives (type, code, value) tuples. It is injected so
    that --selftest can run this with fakes and no /dev/input."""
    for fd in list(trackers.keys()):
        if not is_running():
            return
        tracker = trackers.get(fd)
        if tracker is None:
            continue
        if fd in ready:
            for etype, code, value in read_events(fd):
                if etype == EV_KEY and code == KEY_ESC:
                    if value in (1, 2):
                        tracker.on_key(True, now)
                    elif value == 0:
                        tracker.on_key(False, now)
        if tracker.check_timeout(now):
            on_fire()


def read_events(fd):
    """Every queued input_event on a non-blocking evdev fd."""
    events = []
    while True:
        try:
            data = os.read(fd, EVENT_SIZE)
        except OSError:
            # BlockingIOError: queue empty. ENODEV: unplugged —
            # prune_disconnected() removes it on the next pass.
            break
        if not data or len(data) < EVENT_SIZE:
            break
        _sec, _usec, etype, code, value = struct.unpack(EVENT_FMT, data)
        events.append((etype, code, value))
    return events


# ------------------------------------------------------------------- the daemon

def run_daemon():
    running_flag = {"running": True}

    def handle_stop(signum, frame):
        running_flag["running"] = False

    signal.signal(signal.SIGTERM, handle_stop)
    signal.signal(signal.SIGINT, handle_stop)

    uevent_sock = open_uevent_socket()

    grabbed = {}   # path -> {"fd": fd, "kind": kind}
    trackers = {}  # fd -> EscHoldTracker, keyboards only

    def rescan():
        # prune_disconnected() (see run_daemon's loop, right before this
        # call) has already dropped any path that no longer exists, so
        # "already in grabbed" here means a live device genuinely grabbed
        # already, never a stale entry for one that is long gone and whose
        # path a replugged device might now hold instead.
        for path in sorted(glob.glob("/dev/input/event*")):
            if path in grabbed:
                continue
            info = probe_device(path)
            if info is None:
                continue
            kind = classify_bits(
                info["name"], info["sys_path"], info["key_bits"],
                info["rel_bits"], info["abs_bits"], info["sw_bits"], info["prop_bits"],
            )
            if kind is None:
                continue
            try:
                fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
            except OSError as e:
                log(f"could not open {path}: {e}")
                continue
            try:
                grab(fd)
            except OSError as e:
                log(f"could not grab {path} ({info['name']!r}): {e}")
                os.close(fd)
                continue
            grabbed[path] = {"fd": fd, "kind": kind}
            if kind == "keyboard":
                trackers[fd] = EscHoldTracker()
            log(f"grabbed {path} ({info['name']!r}, {kind})")

    def release_all():
        for path, info in list(grabbed.items()):
            try:
                ungrab(info["fd"])
            except OSError:
                pass
            try:
                os.close(info["fd"])
            except OSError:
                pass
            log(f"released {path}")
        grabbed.clear()
        trackers.clear()

    def attempt_takeover():
        session = discover_session()
        if session is None:
            log("Esc held — no active Wayland session found; keeping the grab")
            return
        user, runtime_dir, wayland_display = session
        try:
            home = pwd.getpwnam(user).pw_dir
        except KeyError:
            log(f"Esc held — user {user!r} has no passwd entry; keeping the grab")
            return
        cmd = build_takeover_cmd(user, runtime_dir, wayland_display, home)
        log(f"Esc held {HOLD_THRESHOLD_SEC:.0f}s — running takeover: {' '.join(cmd)}")

        def run_cmd(timeout):
            return subprocess.run(cmd, timeout=timeout).returncode

        controller = TakeoverController(run_cmd, release_all)
        if controller.attempt():
            log("takeover succeeded — grab released, exiting")
            running_flag["running"] = False

    next_scan = 0.0
    scan_now = True
    try:
        while running_flag["running"]:
            for path in prune_disconnected(grabbed, trackers):
                log(f"{path} disconnected — releasing its grab")
            if scan_now or time.monotonic() >= next_scan:
                rescan()
                next_scan = time.monotonic() + POLL_INTERVAL_SEC
                scan_now = False
            rlist = list(trackers.keys())
            if uevent_sock is not None:
                rlist.append(uevent_sock)
            timeout = max(0.0, next_scan - time.monotonic())
            try:
                ready, _, _ = select.select(rlist, [], [], timeout)
            except (OSError, ValueError):
                ready = []
            if uevent_sock is not None and uevent_sock in ready:
                scan_now = drain_uevents(uevent_sock)
            service_keyboards(
                trackers, ready, time.monotonic(), read_events,
                attempt_takeover, lambda: running_flag["running"],
            )
    finally:
        release_all()
        if uevent_sock is not None:
            uevent_sock.close()


def do_on_stop():
    """`--on-stop` — ExecStopPost's target, run on every stop of
    mango-physlock.service, clean or crashed (SPEC.md §7.2 step 5: fail
    closed). By the time this runs the grab is already gone (systemd only
    runs ExecStopPost once ExecStart's process has exited), so the one
    thing left to do is make sure the session ends locked rather than
    silently unlocked behind panels that may still be dark.

    `lock --until-locked` (not plain `lock`): this only needs to confirm
    the lock started, not block for however long the person takes to type
    their password back in. Best-effort and always returns 0 — a failure
    to lock must not fail the unit's stop, it is already the thing this
    is the last line of defense against."""
    session = discover_session()
    if session is None:
        log("on-stop: no active Wayland session found — nothing to lock")
        return 0
    user, runtime_dir, wayland_display = session
    try:
        home = pwd.getpwnam(user).pw_dir
    except KeyError:
        log(f"on-stop: user {user!r} has no passwd entry")
        return 0
    lock_sh = os.path.join(home, ".config", "mango", "scripts", "sleep-lock.py")
    cmd = user_session_cmd(user, runtime_dir, wayland_display, [lock_sh, "lock", "--until-locked"])
    try:
        ret = subprocess.run(cmd, timeout=20)
        if ret.returncode != 0:
            log("on-stop: lock --until-locked did not confirm a lock")
    except (OSError, subprocess.TimeoutExpired) as e:
        log(f"on-stop: lock attempt failed: {e}")
    return 0


# ------------------------------------------------------------------------ tests

def selftest_classify():
    """classify_bits() is pure — no fd, no /dev/input — so this is the one
    place a wrong exclusion (grabbing the power button, the lid switch, or
    a uinput device) would be caught before it happened on real hardware."""

    def bits(*nums):
        b = bytearray(64)  # must fit BTN_LEFT (0x110 == 272 -> byte 34)
        for n in nums:
            b[n // 8] |= 1 << (n % 8)
        return bytes(b)

    empty = bytes(64)

    assert classify_bits(
        "AT Translated Set 2 keyboard", "/sys/devices/platform/i8042/serio0/input/input1",
        bits(KEY_A), empty, empty, empty, empty,
    ) == "keyboard"

    assert classify_bits(
        "Power Button", "/sys/devices/LNXSYSTM:00/input/input2",
        bits(KEY_POWER), empty, empty, empty, empty,
    ) is None
    assert classify_bits(
        "Some ACPI button", "/sys/devices/x/input/inputN",
        bits(KEY_POWER), empty, empty, empty, empty,
    ) is None, "a device whose only key is KEY_POWER must be skipped regardless of its name"

    assert classify_bits(
        "Lid Switch", "/sys/devices/x/input/inputN",
        empty, empty, empty, bits(SW_LID), empty,
    ) is None

    assert classify_bits(
        "Logitech Mouse", "/sys/devices/pci0000:00/usb1/input/inputN",
        bits(BTN_LEFT), bits(REL_X, REL_Y), empty, empty, empty,
    ) == "mouse"

    assert classify_bits(
        "SynPS/2 Touchpad", "/sys/devices/platform/i8042/serio1/input/inputN",
        bits(BTN_LEFT), empty, bits(ABS_MT_SLOT), empty, empty,
    ) == "touchpad"

    assert classify_bits(
        "ELAN Touchscreen", "/sys/devices/pci0000:00/i2c/input/inputN",
        empty, empty, bits(ABS_MT_SLOT), empty, bits(INPUT_PROP_DIRECT),
    ) == "touchscreen"

    assert classify_bits(
        "kdeconnect virtual keyboard", "/sys/devices/virtual/input/inputN",
        bits(KEY_A), empty, empty, empty, empty,
    ) is None, "a uinput device must never be grabbed, however it classifies"

    # Bluetooth LE keyboards and mice come through BlueZ's uhid, under
    # /sys/devices/virtual/misc/uhid/. They are physical devices: grab them.
    # (Paths from this laptop's real ThinkPad keyboard and RAPOO mouse.)
    assert classify_bits(
        "ThinkPad Compact Bluetooth Keyboard with TrackPoint",
        "/sys/devices/virtual/misc/uhid/0005:17EF:6048.0094/input/input264",
        bits(KEY_A), empty, empty, empty, empty,
    ) == "keyboard", "a Bluetooth (uhid) keyboard must be grabbed"
    assert classify_bits(
        "RAPOO BT4.0 Mouse",
        "/sys/devices/virtual/misc/uhid/0005:000E:3412.0095/input/input265",
        bits(BTN_LEFT), bits(REL_X, REL_Y), empty, empty, empty,
    ) == "mouse", "a Bluetooth (uhid) mouse must be grabbed"

    print("ok: classify_bits")


def selftest_prune_disconnected():
    """`rescan()`'s `if path in grabbed: continue` guard only works if a
    device that unplugged is actually removed from `grabbed` — otherwise a
    replugged keyboard that lands back on the same `/dev/input/eventN`
    (a likely, not a rare, outcome — see `prune_disconnected`'s own
    docstring) is silently never re-grabbed. Driven entirely with a fake
    `grabbed`/`trackers` and a fake `path_exists`: no real /dev/input."""
    closed = []
    grabbed = {
        "/dev/input/event0": {"fd": 10, "kind": "keyboard"},
        "/dev/input/event1": {"fd": 11, "kind": "mouse"},
    }
    trackers = {10: "tracker-for-event0"}
    exists = {"/dev/input/event0": True, "/dev/input/event1": False}

    removed = prune_disconnected(
        grabbed, trackers,
        path_exists=lambda p: exists[p],
        closer=lambda fd: closed.append(fd),
    )

    assert removed == ["/dev/input/event1"], removed
    assert closed == [11], "the disconnected device's fd must be closed"
    assert "/dev/input/event1" not in grabbed, "a vanished path must leave grabbed"
    assert "/dev/input/event0" in grabbed, "a still-present device must be left alone"
    assert 10 in trackers, "a still-present keyboard's tracker must be left alone"
    assert 11 not in trackers, "a vanished keyboard's tracker must be dropped too"

    # The actual failure mode this exists to close: the same path comes
    # back (a reused event-minor number) after being pruned — rescan()'s
    # own "already in grabbed" guard must no longer block it.
    assert "/dev/input/event1" not in grabbed, (
        "a reused path must be free for rescan() to probe and grab again"
    )

    print("ok: prune_disconnected")


def selftest_build_cmd():
    cmd = build_takeover_cmd("martin", "/run/user/1000", "wayland-1", "/home/martin")
    assert cmd == [
        "runuser", "-u", "martin", "--", "env", "XDG_RUNTIME_DIR=/run/user/1000",
        "systemd-run", "--user", "--wait", "--pipe", "--collect", "--quiet",
        "-E", "XDG_RUNTIME_DIR=/run/user/1000", "-E", "WAYLAND_DISPLAY=wayland-1",
        "--", "/home/martin/.config/ironbar/scripts/remote.sh", "--takeover",
    ], cmd
    print("ok: build_takeover_cmd")


def selftest_hold_and_takeover():
    """The one behaviour --selftest must cover per SPEC.md §7.3: a short
    Esc is ignored, a 2s hold triggers exactly one takeover attempt, and
    the grab is kept — never released — on a failed or timed-out attempt,
    with a later attempt still able to succeed (the retry SPEC.md
    describes as "allow a retry"). All driven with fakes: no /dev/input,
    no real runuser or remote.sh call."""

    # --- short Esc ignored ---------------------------------------------
    t = EscHoldTracker()
    t.on_key(True, 0.0)
    t.on_key(False, 1.0)  # released well under the threshold
    assert t.check_timeout(1.0) is False
    assert t.check_timeout(5.0) is False, "a released key must never fire later"

    # --- 2s Esc triggers takeover, exactly once per hold ----------------
    t = EscHoldTracker()
    t.on_key(True, 10.0)
    assert t.check_timeout(10.0 + HOLD_THRESHOLD_SEC - 0.1) is False, "fired before the threshold"
    assert t.check_timeout(10.0 + HOLD_THRESHOLD_SEC) is True, "did not fire at the threshold"
    assert t.check_timeout(10.0 + HOLD_THRESHOLD_SEC + 0.1) is False, "fired twice for one hold"

    calls = []
    controller = TakeoverController(
        run_cmd=lambda timeout: 0,
        release_grab=lambda: calls.append("released"),
    )
    assert controller.attempt() is True
    assert calls == ["released"], "a successful takeover must release the grab"

    # --- grab kept when the takeover command fails ----------------------
    calls = []
    controller = TakeoverController(
        run_cmd=lambda timeout: 1,
        release_grab=lambda: calls.append("released"),
    )
    assert controller.attempt() is False
    assert calls == [], "a failed takeover must not release the grab"

    # --- grab kept when the takeover command times out ------------------
    def raise_timeout(timeout):
        raise subprocess.TimeoutExpired(cmd=["remote.sh", "--takeover"], timeout=timeout)

    calls = []
    controller = TakeoverController(run_cmd=raise_timeout, release_grab=lambda: calls.append("released"))
    assert controller.attempt() is False
    assert calls == [], "a timed-out takeover must not release the grab"

    # --- a kept grab allows a retry: the same controller, tried again,
    # releases once the command finally succeeds.
    controller.run_cmd = lambda timeout: 0
    assert controller.attempt() is True
    assert calls == ["released"]

    print("ok: hold detector + takeover ordering")


def selftest_service_keyboards():
    """A successful takeover from one keyboard clears every tracker while
    service_keyboards() is still in its pass over two keyboards. The pass
    must stop cleanly (no KeyError), so the daemon exits 0. Fakes only."""
    running = {"on": True}
    trackers = {10: EscHoldTracker(), 11: EscHoldTracker()}
    fired = []

    def on_fire():
        fired.append("takeover")
        trackers.clear()           # what release_all() does
        running["on"] = False      # what attempt_takeover() does on success

    # Esc went down on keyboard 10 at t=0 and is still down.
    trackers[10].on_key(True, 0.0)
    service_keyboards(
        trackers, ready=[10, 11], now=HOLD_THRESHOLD_SEC,
        read_events=lambda fd: [], on_fire=on_fire,
        is_running=lambda: running["on"],
    )
    assert fired == ["takeover"], fired

    # Same case, but the daemon keeps running (a failed takeover clears
    # nothing) — the second keyboard is still served.
    trackers = {10: EscHoldTracker(), 11: EscHoldTracker()}
    fired = []
    service_keyboards(
        trackers, ready=[11], now=0.0,
        read_events=lambda fd: [(EV_KEY, KEY_ESC, 1)],
        on_fire=lambda: fired.append("x"), is_running=lambda: True,
    )
    assert trackers[11]._down_since == 0.0, "Esc on keyboard 11 was not seen"
    assert trackers[10]._down_since is None, "keyboard 10 was not ready"
    service_keyboards(
        trackers, ready=[], now=HOLD_THRESHOLD_SEC,
        read_events=lambda fd: [], on_fire=lambda: fired.append("x"),
        is_running=lambda: True,
    )
    assert fired == ["x"], "a hold must fire from the poll wakeup with no new event"

    print("ok: service_keyboards")


def selftest_drain_uevents():
    """drain_uevents() asks for a rescan only on an input "add" event."""

    class FakeSock:
        def __init__(self, msgs):
            self.msgs = list(msgs)

        def recv(self, n):
            if not self.msgs:
                raise BlockingIOError
            return self.msgs.pop(0)

    add_input = b"add@/devices/x/input/input9\x00ACTION=add\x00SUBSYSTEM=input\x00"
    change_power = b"change@/devices/x/power_supply/BAT0\x00ACTION=change\x00SUBSYSTEM=power_supply\x00"
    assert drain_uevents(FakeSock([change_power])) is False
    assert drain_uevents(FakeSock([change_power, add_input])) is True
    print("ok: drain_uevents")


def selftest():
    selftest_classify()
    selftest_prune_disconnected()
    selftest_build_cmd()
    selftest_hold_and_takeover()
    selftest_service_keyboards()
    selftest_drain_uevents()
    print("ok")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--selftest":
        selftest()
        sys.exit(0)
    if len(sys.argv) > 1 and sys.argv[1] == "--on-stop":
        sys.exit(do_on_stop())
    if os.geteuid() != 0:
        sys.exit("mango-physlock: must run as root (EVIOCGRAB needs it)")
    run_daemon()
