//! Minute-granular clock, replacing clock.sh's 1Hz poll (its self-described
//! ponytail note: "timer only advances while waybar polls"). One absolute
//! timerfd wakeup/min is exact, not approximate, because the face is HH:MM.
//!
//! Uses libc's localtime_r/strftime (glibc calls tzset() internally on every
//! localtime_r, so TZ changes across suspend/resume are picked up for free)
//! rather than a date-handling crate — this is the one place in the crate
//! doing calendar math, and libc is already a dependency for the timerfd
//! syscalls.

use crate::sys::ClockTimer;
use crate::vars::Vars;
use std::os::fd::{AsRawFd, RawFd};

pub struct Clock {
    timer: ClockTimer,
}

impl AsRawFd for Clock {
    fn as_raw_fd(&self) -> RawFd {
        self.timer.as_raw_fd()
    }
}

impl Clock {
    pub fn new() -> std::io::Result<Self> {
        let timer = ClockTimer::new()?;
        timer.arm_next_minute()?;
        Ok(Self { timer })
    }

    /// Call after the fd is readable. Always refreshes+re-arms; on a
    /// discontinuous clock change (ECANCELED) the timer comes back disarmed,
    /// so re-arming here is required, not optional.
    pub fn on_tick(&self, vars: &mut Vars) {
        let _ = self.timer.drain();
        let _ = self.timer.arm_next_minute();
        refresh(vars);
    }
}

pub fn refresh(vars: &mut Vars) {
    vars.set("clk_text", format_local("%H:%M"));
    vars.set("date_text", format_local("%a, %d %b"));
}

fn format_local(fmt: &str) -> String {
    // SAFETY: `t` is a valid time_t from libc::time; `tm` is a zeroed,
    // correctly-sized out-param filled in place by localtime_r; `cfmt` is a
    // NUL-terminated CString kept alive through the strftime call; `buf` is
    // sized generously (128 bytes) for the short formats this crate uses,
    // and strftime's return value (bytes written) is used to truncate it —
    // never trusted as a length beyond what strftime itself reports.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let cfmt = std::ffi::CString::new(fmt).expect("format has no NUL bytes");
        let mut buf = vec![0u8; 128];
        let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm);
        buf.truncate(n);
        String::from_utf8_lossy(&buf).into_owned()
    }
}
