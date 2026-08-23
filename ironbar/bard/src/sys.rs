//! The only unsafe in this crate: timerfd, inotify, prctl. Kernel ABI
//! constants are defined locally rather than trusting a specific `libc`
//! version to export them (timerfd/inotify are Linux-only and some flags,
//! e.g. TFD_TIMER_CANCEL_ON_SET, have been inconsistently exposed across
//! libc releases) — the numeric values are stable kernel UAPI, not
//! implementation details.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

const CLOCK_REALTIME: libc::clockid_t = 0;
const CLOCK_MONOTONIC: libc::clockid_t = 1;
const TFD_NONBLOCK: libc::c_int = libc::O_NONBLOCK;
const TFD_CLOEXEC: libc::c_int = libc::O_CLOEXEC;
const TFD_TIMER_ABSTIME: libc::c_int = 1 << 0;
const TFD_TIMER_CANCEL_ON_SET: libc::c_int = 1 << 1;

const IN_CLOSE_WRITE: u32 = 0x0000_0008;
const IN_MOVE_SELF: u32 = 0x0000_0800;
const IN_DELETE_SELF: u32 = 0x0000_0400;
const IN_NONBLOCK: libc::c_int = libc::O_NONBLOCK;
const IN_CLOEXEC: libc::c_int = libc::O_CLOEXEC;

fn check(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// A `timerfd` armed absolutely on `CLOCK_REALTIME` with
/// `TFD_TIMER_CANCEL_ON_SET`, so a discontinuous clock change (NTP step,
/// suspend/resume) makes the next `read()` fail with `ECANCELED` instead of
/// silently drifting. Caller re-arms on that error — see clock.rs.
pub struct ClockTimer {
    fd: OwnedFd,
}

impl AsRawFd for ClockTimer {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl ClockTimer {
    pub fn new() -> io::Result<Self> {
        // SAFETY: timerfd_create with valid, locally-defined flag constants;
        // no pointers involved. Return value is checked before use.
        let raw =
            check(unsafe { libc::timerfd_create(CLOCK_REALTIME, TFD_NONBLOCK | TFD_CLOEXEC) })?;
        // SAFETY: `raw` is a just-created, valid, owned fd from timerfd_create above.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self { fd })
    }

    /// Arms (or re-arms) for the next minute boundary, repeating every 60s.
    pub fn arm_next_minute(&self) -> io::Result<()> {
        let now = unsafe {
            let mut ts: libc::timespec = std::mem::zeroed();
            // SAFETY: `ts` is a valid, properly-sized out-param for clock_gettime.
            check(libc::clock_gettime(CLOCK_REALTIME, &mut ts))?;
            ts.tv_sec
        };
        let next = (now / 60 + 1) * 60;
        let spec = libc::itimerspec {
            it_interval: libc::timespec {
                tv_sec: 60,
                tv_nsec: 0,
            },
            it_value: libc::timespec {
                tv_sec: next,
                tv_nsec: 0,
            },
        };
        // SAFETY: `self.fd` is a valid timerfd for this process; `spec` is a
        // fully-initialized itimerspec; old_value out-param is null (we don't
        // need the previous setting).
        check(unsafe {
            libc::timerfd_settime(
                self.fd.as_raw_fd(),
                TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET,
                &spec,
                std::ptr::null_mut(),
            )
        })?;
        Ok(())
    }

    /// Arms a one-shot absolute wakeup at `epoch_secs` (`it_interval` zero —
    /// unlike `arm_next_minute`, phase lengths vary, so the caller re-arms
    /// explicitly at the next boundary rather than repeating on a fixed
    /// period). Same `TFD_TIMER_CANCEL_ON_SET` guard: a manual clock set
    /// during a phase does not silently retarget the deadline, it just
    /// forces the same re-arm-on-ECANCELED path `drain()` already documents.
    pub fn arm_at(&self, epoch_secs: i64) -> io::Result<()> {
        let spec = libc::itimerspec {
            it_interval: libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: libc::timespec {
                tv_sec: epoch_secs as libc::time_t,
                tv_nsec: 0,
            },
        };
        // SAFETY: `self.fd` is a valid timerfd for this process; `spec` is a
        // fully-initialized itimerspec; old_value out-param is null.
        check(unsafe {
            libc::timerfd_settime(
                self.fd.as_raw_fd(),
                TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET,
                &spec,
                std::ptr::null_mut(),
            )
        })?;
        Ok(())
    }

    /// Disarms without closing the fd (`it_value` zero, per
    /// `timerfd_settime(2)`) — used when a pomodoro is paused or reset, so
    /// the phase-boundary fd goes quiet exactly like `PollTimer`'s own
    /// zero-period disarm.
    pub fn disarm(&self) -> io::Result<()> {
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let spec = libc::itimerspec {
            it_interval: zero,
            it_value: zero,
        };
        // SAFETY: same as `arm_at` above.
        check(unsafe {
            libc::timerfd_settime(self.fd.as_raw_fd(), 0, &spec, std::ptr::null_mut())
        })?;
        Ok(())
    }

    /// Drains the expiration counter. `Ok(None)` means the clock was reset
    /// (ECANCELED) and the timer is now disarmed — caller must re-arm.
    pub fn drain(&self) -> io::Result<Option<u64>> {
        let mut buf = [0u8; 8];
        // SAFETY: `buf` is an 8-byte buffer matching timerfd's fixed read
        // size (a u64 expiration counter); `self.fd` is a valid timerfd.
        let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut _, 8) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ECANCELED) {
                return Ok(None);
            }
            if e.kind() == io::ErrorKind::WouldBlock {
                return Ok(Some(0));
            }
            return Err(e);
        }
        Ok(Some(u64::from_ne_bytes(buf)))
    }
}

/// inotify watch on exactly one file's inode (never a directory — see
/// IRONBAR.md Spike findings: waybar's script children generate ~1000
/// tempfile events/min on $XDG_RUNTIME_DIR, which a directory watch would
/// pay for on every wakeup).
pub struct FileWatch {
    fd: OwnedFd,
    wd: Option<libc::c_int>,
    path: std::ffi::CString,
}

impl AsRawFd for FileWatch {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl FileWatch {
    pub fn new(path: &str) -> io::Result<Self> {
        // SAFETY: valid, locally-defined flag constants; no pointers.
        let raw = check(unsafe { libc::inotify_init1(IN_NONBLOCK | IN_CLOEXEC) })?;
        // SAFETY: `raw` is a just-created, valid, owned fd from inotify_init1 above.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let path = std::ffi::CString::new(path).expect("path has no NUL bytes");
        let mut w = Self { fd, wd: None, path };
        w.rewatch();
        Ok(w)
    }

    /// (Re)establishes the watch. Safe to call when the file doesn't exist
    /// yet or the previous watch was invalidated (IN_IGNORED) — failure is
    /// silent and retried lazily; the clock's minute tick guarantees a retry
    /// at least once a minute even if every mode-change event is missed.
    pub fn rewatch(&mut self) {
        let mask = IN_CLOSE_WRITE | IN_MOVE_SELF | IN_DELETE_SELF;
        // SAFETY: `self.fd` is a valid inotify fd; `self.path` is a
        // NUL-terminated CString kept alive for the duration of the call.
        let wd = unsafe { libc::inotify_add_watch(self.fd.as_raw_fd(), self.path.as_ptr(), mask) };
        self.wd = if wd >= 0 { Some(wd) } else { None };
    }

    /// Drains pending events (we don't care which — any event means
    /// "re-read the file"), returning whether anything fired.
    pub fn drain(&mut self) -> bool {
        let mut buf = [0u8; 1024];
        let mut any = false;
        loop {
            // SAFETY: `buf` is a valid, sufficiently large buffer for
            // inotify_event records; `self.fd` is a valid inotify fd.
            let n =
                unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            any = true;
        }
        if self.wd.is_none() {
            self.rewatch();
        }
        any
    }
}

/// A `timerfd` on `CLOCK_MONOTONIC`, relative and disarmable — the poll
/// wheel's clock (T6a: cpu.rs/memory.rs, see wheel.rs). Unlike `ClockTimer`
/// (absolute, `CLOCK_REALTIME`, zero slack by kernel design) this is a plain
/// interval timer, so `set_timer_slack` below actually coalesces its wakeups
/// with the rest of the system's — this is the timer that doc comment's
/// "matters once a later stage adds relative-interval polls" was about.
/// Arming with a zero period disarms the timer without closing the fd
/// (per `timerfd_settime(2)`), which is how eco mode costs nothing.
pub struct PollTimer {
    fd: OwnedFd,
}

impl AsRawFd for PollTimer {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl PollTimer {
    pub fn new() -> io::Result<Self> {
        // SAFETY: timerfd_create with valid, locally-defined flag constants;
        // no pointers involved. Return value is checked before use.
        let raw =
            check(unsafe { libc::timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK | TFD_CLOEXEC) })?;
        // SAFETY: `raw` is a just-created, valid, owned fd from timerfd_create above.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self { fd })
    }

    /// Arms a repeating relative timer at `period`. `period` of zero disarms
    /// the timer (per `timerfd_settime(2)`) without closing the fd.
    pub fn arm(&self, period: Duration) -> io::Result<()> {
        let ts = libc::timespec {
            tv_sec: period.as_secs() as libc::time_t,
            tv_nsec: libc::c_long::from(period.subsec_nanos()),
        };
        let spec = libc::itimerspec {
            it_interval: ts,
            it_value: ts,
        };
        // SAFETY: `self.fd` is a valid timerfd for this process; `spec` is a
        // fully-initialized itimerspec (relative, no TFD_TIMER_ABSTIME);
        // old_value out-param is null (we don't need the previous setting).
        check(unsafe {
            libc::timerfd_settime(self.fd.as_raw_fd(), 0, &spec, std::ptr::null_mut())
        })?;
        Ok(())
    }

    /// Drains the expiration counter. `WouldBlock` (nothing pending — e.g.
    /// the timer is disarmed) reads as 0 ticks, not an error.
    pub fn drain(&self) -> io::Result<u64> {
        let mut buf = [0u8; 8];
        // SAFETY: `buf` is an 8-byte buffer matching timerfd's fixed read
        // size (a u64 expiration counter); `self.fd` is a valid timerfd.
        let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut _, 8) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::WouldBlock {
                return Ok(0);
            }
            return Err(e);
        }
        Ok(u64::from_ne_bytes(buf))
    }
}

/// Coalesces this process's relative timers with the kernel's choosing.
/// Does NOT affect the absolute clock timerfd (TFD_TIMER_ABSTIME timers get
/// zero slack by kernel design) — matters once a later stage adds
/// relative-interval polls (cpu/mem).
pub fn set_timer_slack(ms: u64) {
    // SAFETY: PR_SET_TIMERSLACK takes a single unsigned-long nanosecond
    // argument and has no failure mode that affects memory safety.
    unsafe {
        libc::prctl(libc::PR_SET_TIMERSLACK, ms.saturating_mul(1_000_000));
    }
}
