//! The poll wheel: one relative timerfd shared by cpu.rs and memory.rs, the
//! only two honest polls left in the daemon — no kernel event reports "CPU
//! is now 47% busy" (IRONBAR.md T6a). Owns the period table so neither
//! collector matches on `powermode::Mode` for scheduling.
//!
//! Eco mode disarms the wheel outright rather than following IRONBAR.md's
//! originally documented 30s/60s eco periods: honoring that table would add
//! 2+ wakeups/min on its own and break goal 2's `<=2 wakeups/min` eco
//! invariant on the first stage that touches it. Instead cpu/memory ride the
//! existing minute clock tick in eco, exactly as net.rs's RSSI gate already
//! does — see `on_minute`.
//!
//! The battery-mode memory period (originally documented as 15s) is rounded
//! to 20s: one timerfd with one interval means memory's period must be an
//! integer multiple of cpu's, and 2 is the nearest divisor to 15/10.

use crate::powermode;
use crate::sys::PollTimer;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

/// Memory refreshes on every `MEM_EVERY`th wheel tick; cpu on every one.
const MEM_EVERY: u32 = 2;

fn period_for(mode: powermode::Mode) -> Duration {
    match mode {
        powermode::Mode::Full => Duration::from_secs(5),
        powermode::Mode::Battery => Duration::from_secs(10),
        powermode::Mode::Eco => Duration::ZERO, // disarmed — see module doc
    }
}

pub struct Due {
    pub cpu: bool,
    pub mem: bool,
}

pub struct Wheel {
    timer: PollTimer,
    tick: u32,
    period: Duration,
}

impl AsRawFd for Wheel {
    fn as_raw_fd(&self) -> RawFd {
        self.timer.as_raw_fd()
    }
}

impl Wheel {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            timer: PollTimer::new()?,
            tick: 0,
            period: Duration::ZERO,
        })
    }

    /// Re-programs the wheel for `mode`. Returns whether the period actually
    /// changed, so callers only log/reset on a real transition (IRONBAR.md
    /// T6 acceptance: "log line per re-program").
    pub fn set_mode(&mut self, mode: powermode::Mode) -> std::io::Result<bool> {
        let period = period_for(mode);
        if period == self.period {
            return Ok(false);
        }
        self.timer.arm(period)?;
        self.period = period;
        self.tick = 0;
        Ok(true)
    }

    /// Drains the timer and reports what's due. Call after the wheel fd is
    /// readable (full/battery only — the timer is disarmed in eco, so this
    /// never fires there). cpu is due every tick; mem every `MEM_EVERY`th.
    pub fn on_tick(&mut self) -> Due {
        let _ = self.timer.drain();
        self.tick = self.tick.wrapping_add(1);
        Due {
            cpu: true,
            mem: self.tick.is_multiple_of(MEM_EVERY),
        }
    }

    /// Call on the minute clock tick. Only eco mode rides it — full/battery
    /// already get more frequent updates from the wheel itself.
    pub fn on_minute(&self, mode: powermode::Mode) -> Due {
        let due = mode == powermode::Mode::Eco;
        Due { cpu: due, mem: due }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_table_matches_t6a_design() {
        assert_eq!(period_for(powermode::Mode::Full), Duration::from_secs(5));
        assert_eq!(
            period_for(powermode::Mode::Battery),
            Duration::from_secs(10)
        );
        assert_eq!(period_for(powermode::Mode::Eco), Duration::ZERO);
    }

    #[test]
    fn mem_rides_every_second_wheel_tick() {
        let mut w = Wheel::new().expect("timerfd_create");
        let d1 = w.on_tick();
        assert!(d1.cpu && !d1.mem);
        let d2 = w.on_tick();
        assert!(d2.cpu && d2.mem);
        let d3 = w.on_tick();
        assert!(d3.cpu && !d3.mem);
    }

    #[test]
    fn on_minute_only_fires_in_eco() {
        let w = Wheel::new().expect("timerfd_create");
        let full = w.on_minute(powermode::Mode::Full);
        assert!(!full.cpu && !full.mem);
        let battery = w.on_minute(powermode::Mode::Battery);
        assert!(!battery.cpu && !battery.mem);
        let eco = w.on_minute(powermode::Mode::Eco);
        assert!(eco.cpu && eco.mem);
    }

    #[test]
    fn set_mode_reports_no_change_on_repeat() {
        let mut w = Wheel::new().expect("timerfd_create");
        assert!(w.set_mode(powermode::Mode::Full).expect("arm"));
        assert!(!w.set_mode(powermode::Mode::Full).expect("arm"));
        assert!(w.set_mode(powermode::Mode::Battery).expect("arm"));
    }

    #[test]
    fn set_mode_resets_tick_counter_on_transition() {
        let mut w = Wheel::new().expect("timerfd_create");
        w.set_mode(powermode::Mode::Full).expect("arm");
        w.on_tick();
        w.set_mode(powermode::Mode::Battery).expect("arm");
        // A fresh mode starts its own cadence: the first tick after a
        // transition must not land on an old, unrelated MEM_EVERY boundary.
        let d = w.on_tick();
        assert!(d.cpu && !d.mem);
    }
}
