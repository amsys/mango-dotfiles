//! Pomodoro engine — T7c, and the FOCUS.md build-out (see
//! /home/martin/.claude/plans/read-focus-md-implement-all-mossy-widget.md).
//! Ports src/ironbar/scripts/clock.sh's pomodoro state machine (`phase_len`,
//! `next_phase`, `tick`, `muted`, `nag`), which that script's own header
//! called out as ponytail: "the timer only advances while waybar is
//! polling". Here the fix is real: `tick()`'s phase-boundary check is driven
//! by one absolute timerfd per IRONBAR.md's timer-discipline rule ("the
//! pomodoro arms one absolute timer at the next phase boundary, not a 1 Hz
//! tick"), not a poll, and the on-screen countdown is minute-granular,
//! riding the existing clock tick instead of ticking every second.
//!
//! `clock.sh`'s own pomodoro state machine was never ported in parallel —
//! this is the only implementation. `clock.sh` itself survives the T8
//! cutover for its clock/date/calendar half (moved, not deleted).
//!
//! FOCUS.md additions beyond clock.sh's original engine: a named task per
//! work block (§1.3), a Ready-to-Resume prompt at the work bell when a task
//! was named (§2.1), a parking-lot distraction tally (§2.2/§5.4), mako
//! do-not-disturb for the duration of a work block (§5.3), and a per-phase
//! log line for the weekly review (§5.6).

use crate::clock::format_epoch;
use crate::cmd::run;
use crate::mango::CLASS_PREFIX;
use crate::remote;
use crate::sys::ClockTimer;
use crate::tooltip;
use crate::vars::Vars;
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use tokio::process::Command;

/// Nerd Font MDI "timer_outline" — the same glyph clock.sh's own tooltip
/// already used for its Pomodoro section (IC_POMO, U+F051B). Reused rather
/// than clock.sh's *bar* icon (Material Symbols Rounded): T8b found that
/// font fails to rasterize under this GTK4/ironbar setup, while the Nerd
/// Font MDI family clock.sh's tooltip already used, and darkmode.rs's T8b
/// fix confirmed render correctly, does not have that problem.
const IC_TIMER: char = '\u{f051b}';
/// audio.rs's old `IC_MIC_MUTED` value — reused verbatim (not exported, so
/// redefined here) rather than picking an unverified codepoint: this glyph
/// was already proven to rasterize in this exact ironbar instance.
/// T19: U+F131 -> U+F09A2 (md-bell_off) — one-icon-family sweep (IRONBAR.md
/// T19). This pill mutes *notifications*, not a microphone, so bell_off also
/// reads correctly rather than reusing a mic glyph's own on/off pair for an
/// unrelated meaning; audio.rs's `IC_MIC_MUTED` moved to its own md glyph in
/// the same sweep, so the two no longer collide anyway.
const IC_MUTED: char = '\u{f09a2}';

// ponytail: one glyph for both work and break, state carried by the CSS
// class (`@class/pomo`) and the phase label text, same minimalism net.rs's
// spinner doc comment argues for its own single glyph. Add a second icon
// once a break-specific Nerd Font MDI codepoint is verified to render here.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Work,
    Short,
    Long,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Work => "Focus",
            Phase::Short => "Short break",
            Phase::Long => "Long break",
        }
    }
    fn css(self) -> &'static str {
        match self {
            Phase::Work => "work",
            Phase::Short => "short",
            Phase::Long => "long",
        }
    }
    fn log_name(self) -> &'static str {
        match self {
            Phase::Work => "work",
            Phase::Short => "short-break",
            Phase::Long => "long-break",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Run {
    Idle,
    Running,
    Pause,
}

struct Config {
    work_min: i64,
    short_min: i64,
    long_min: i64,
    cycle: u32,
    mute_min: i64,
}

impl Config {
    fn from_env() -> Self {
        let raw = std::env::var("MANGO_POMODORO").unwrap_or_else(|_| "25,5,15,4".into());
        let mut p = raw.split(',').map(|s| s.trim().parse::<i64>().unwrap_or(0));
        Self {
            work_min: p.next().filter(|&n| n > 0).unwrap_or(25),
            short_min: p.next().filter(|&n| n > 0).unwrap_or(5),
            long_min: p.next().filter(|&n| n > 0).unwrap_or(15),
            cycle: p
                .next()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n > 0)
                .unwrap_or(4),
            mute_min: std::env::var("MANGO_POMODORO_MUTE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
        }
    }

    fn phase_len(&self, p: Phase) -> i64 {
        60 * match p {
            Phase::Work => self.work_min,
            Phase::Short => self.short_min,
            Phase::Long => self.long_min,
        }
    }

    /// clock.sh:93-97 `next_phase()`. `completed` is the work-block count
    /// *after* the block that just ended has been banked (see `on_boundary`
    /// below, which increments before calling this — matching clock.sh:217's
    /// own `count=$((count + 1))` ahead of its `next_phase` call).
    fn next_phase(&self, p: Phase, completed: u32) -> Phase {
        match p {
            Phase::Work if completed.is_multiple_of(self.cycle) => Phase::Long,
            Phase::Work => Phase::Short,
            _ => Phase::Work,
        }
    }
}

// clock.sh:39-41 — kept as the same three named constants, now in seconds
// throughout rather than clock.sh's mixed seconds/minutes. Nag runs off the
// minute clock tick here (see `on_minute`), not a 1 Hz poll, so the grace
// window is rounded up to a whole minute: clock.sh's 45s grace and this
// port's 60s grace both mean "warn on the first tick after the break
// starts, once it's had a moment to register" — the nearest a minute-tick
// cadence can get to 45s without a timer of its own.
const NAG_GRACE_SECS: i64 = 60;
const NAG_EVERY_SECS: i64 = 60;
const NAG_ESCALATE_SECS: i64 = 300;

pub struct Pomo {
    timer: ClockTimer,
    cfg: Config,
    state: Run,
    phase: Phase,
    /// Deadline epoch while `Run`; remaining seconds while `Pause` — same
    /// dual convention as clock.sh's own `$UNTIL` (clock.sh:400-401), kept
    /// so the ported test fixtures below check the same arithmetic.
    until: i64,
    phase_start: i64,
    count: u32,
    task: Option<String>,
    distractions: u32,
    resume: Option<String>,
    mute_until: Option<i64>,
    nag_first: Option<i64>,
    nag_last: Option<i64>,
    idle_flag: bool,
}

fn now() -> i64 {
    // SAFETY: libc::time never fails (no error return per POSIX).
    unsafe { libc::time(std::ptr::null_mut()) }
}

fn runtime_state_path() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("mango-bard-pomo")
}

/// `$XDG_DATA_HOME/mango-focus` (falling back to `~/.local/share`) — the
/// parking lot, resume notes and the weekly log all live here; the helper
/// scripts (focus-note.sh etc.) write the same directory directly.
fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(d).join("mango-focus");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/share/mango-focus")
}

impl AsRawFd for Pomo {
    fn as_raw_fd(&self) -> RawFd {
        self.timer.as_raw_fd()
    }
}

impl Pomo {
    pub fn new() -> std::io::Result<Self> {
        let timer = ClockTimer::new()?;
        let mut p = Self {
            timer,
            cfg: Config::from_env(),
            state: Run::Idle,
            phase: Phase::Work,
            until: 0,
            phase_start: 0,
            count: 0,
            task: None,
            distractions: 0,
            resume: None,
            mute_until: None,
            nag_first: None,
            nag_last: None,
            idle_flag: false,
        };
        p.load();
        if p.state == Run::Running {
            let _ = p.timer.arm_at(p.until);
        }
        Ok(p)
    }

    /// Restart survival: if the daemon was down past one or more phase
    /// boundaries, catch up now rather than leaving a stale deadline. Called
    /// once from main.rs's startup sequence, same shape as `cpu.prime()` —
    /// separate from `new()` because catching up needs `.await` (it may
    /// re-arm the timer) and `new()` itself must stay sync, matching every
    /// other collector's own `X::new()` here.
    pub async fn catch_up_on_start(&mut self) {
        if self.state != Run::Running {
            return;
        }
        self.catch_up(now(), false).await;
        if self.state == Run::Running {
            let _ = self.timer.arm_at(self.until);
        }
    }

    // ---------------------------------------------------------- persistence

    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(runtime_state_path()) else {
            return;
        };
        let mut lines = text.lines();
        let state = match lines.next() {
            Some("run") => Run::Running,
            Some("pause") => Run::Pause,
            _ => return,
        };
        let phase = match lines.next() {
            Some("short") => Phase::Short,
            Some("long") => Phase::Long,
            _ => Phase::Work,
        };
        let until: i64 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let phase_start: i64 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: u32 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let distractions: u32 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let task = lines.next().filter(|s| !s.is_empty()).map(str::to_string);
        self.state = state;
        self.phase = phase;
        self.until = until;
        self.phase_start = phase_start;
        self.count = count;
        self.distractions = distractions;
        self.task = task;
        self.clamp_until(now()); // a stale or torn file can carry an out-of-range value
    }

    fn save(&self) {
        let state = match self.state {
            Run::Idle => {
                let _ = std::fs::remove_file(runtime_state_path());
                return;
            }
            Run::Running => "run",
            Run::Pause => "pause",
        };
        let phase = match self.phase {
            Phase::Work => "work",
            Phase::Short => "short",
            Phase::Long => "long",
        };
        let text = format!(
            "{state}\n{phase}\n{}\n{}\n{}\n{}\n{}\n",
            self.until,
            self.phase_start,
            self.count,
            self.distractions,
            self.task.as_deref().unwrap_or(""),
        );
        let _ = std::fs::write(runtime_state_path(), text);
    }

    fn log_path() -> PathBuf {
        data_dir().join("log.tsv")
    }

    /// One TSV row per finished phase: date, start, end, phase, minutes,
    /// task, distractions — FOCUS.md §5.6's weekly review reads exactly
    /// these seven columns.
    fn log_phase(&self, ended: Phase, start: i64, end: i64) {
        let dir = data_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let line = format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            format_epoch(start, "%Y-%m-%d"),
            format_epoch(start, "%H:%M"),
            format_epoch(end, "%H:%M"),
            ended.log_name(),
            (end - start) / 60,
            self.task.as_deref().unwrap_or(""),
            self.distractions,
        );
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Self::log_path())
        {
            let _ = f.write_all(line.as_bytes());
        }
    }

    // ---------------------------------------------------------------- pure

    /// clock.sh:184-189 `muted()` — the deadline-straddle convention (`<`,
    /// not `<=`) is load-bearing, see the ported test below.
    fn muted_at(&self, now: i64) -> bool {
        self.mute_until.is_some_and(|end| now < end)
    }

    // ------------------------------------------------------------- actions

    /// Called once per minute clock tick (see main.rs's clock-fd arm) while
    /// `Run` or `Pause` — advances the countdown display and, for a break,
    /// the nag warning. Never called while `Idle`: nothing to show.
    pub async fn on_minute(&mut self, vars: &mut Vars) {
        if self.state == Run::Idle {
            return;
        }
        let n = now();
        if self.state == Run::Running && self.phase != Phase::Work {
            self.nag(n).await;
        }
        self.refresh(vars);
    }

    /// clock.sh:253-266 `nag()`, ported 1:1 except the grace/repeat/escalate
    /// windows are now whole minutes (see the module doc's `NAG_*` comment)
    /// and the idle signal is `self.idle_flag` (set by `pomo idle 0|1`,
    /// itself driven by hypridle — see hypridle.conf) rather than a file
    /// this poll used to `stat`.
    async fn nag(&mut self, n: i64) {
        if self.idle_flag {
            self.nag_first = None;
            self.nag_last = None;
            return;
        }
        if n - self.phase_start < NAG_GRACE_SECS {
            return;
        }
        let first = *self.nag_first.get_or_insert(n);
        let last = self.nag_last.unwrap_or(0);
        if n - last < NAG_EVERY_SECS {
            return;
        }
        self.nag_last = Some(n);
        self.notify(
            "critical",
            "Still working",
            &format!("{} \u{2022} step away", self.phase.label()),
        )
        .await;
        if n - first >= NAG_ESCALATE_SECS {
            self.sound("message");
        }
    }

    /// Fires when the phase-boundary timerfd becomes readable.
    pub async fn on_boundary(&mut self, vars: &mut Vars) {
        match self.timer.drain() {
            Ok(None) => {
                // Clock stepped (ECANCELED) — `until` is still the right
                // wall-clock target, only the kernel's tracking needs a
                // reset. Re-arm at the same deadline, no state change.
                if self.state == Run::Running {
                    let _ = self.timer.arm_at(self.until);
                }
                return;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("mango-bard: pomo timer error: {e}");
                return;
            }
        }
        if self.state != Run::Running {
            return;
        }
        self.catch_up(now(), true).await;
        if self.state == Run::Running {
            let _ = self.timer.arm_at(self.until);
        }
        self.save();
        self.refresh(vars);
    }

    /// Advances across every phase boundary `now` has already passed —
    /// normally exactly one (the timer fires right at the deadline), but a
    /// daemon restart after a long gap (or a suspend) can leave several
    /// behind, so this loops rather than clock.sh's single-step `if`
    /// (clock.sh:216), which relied on being re-polled every second to
    /// catch up. `fire_alerts: false` means "starting up, nothing to alert
    /// on yet" — `catch_up_on_start`'s own call, which must not fire bells
    /// for phases that ended while the daemon wasn't running.
    async fn catch_up(&mut self, n: i64, fire_alerts: bool) {
        let mut guard = 0;
        while self.until <= n && guard < 1000 {
            guard += 1;
            let ended = self.phase;
            let start = self.phase_start;
            let end = self.until;
            if ended == Phase::Work {
                self.count += 1;
            }
            let next = self.cfg.next_phase(ended, self.count);
            self.phase = next;
            self.phase_start = end;
            self.until = end + self.cfg.phase_len(next);
            self.nag_first = None;
            self.nag_last = None;
            self.log_phase(ended, start, end);
            if fire_alerts {
                self.alert(ended, next).await;
            }
        }
    }

    /// Everything a phase boundary triggers besides the state transition
    /// itself: sound, the full-screen overlay, DND, the resume prompt and
    /// the distraction-tally reset — all funnelled through here so no
    /// future caller can bypass the mute guard, same rationale clock.sh:
    /// 191-192 gave for its own single `notify()`/`beep()` chokepoint.
    async fn alert(&mut self, ended: Phase, next: Phase) {
        if next == Phase::Work {
            self.distractions = 0;
        }
        if ended == Phase::Work {
            self.release_dnd().await;
            if let Some(task) = self.task.take() {
                spawn_detached("focus-resume.sh", &[&task]);
            }
            self.notify(
                "critical",
                "Pomodoro done",
                &format!("{} \u{2022} sip water", next.label()),
            )
            .await;
            self.sound("complete");
            spawn_detached(
                "focus-break.sh",
                &[next.label(), &(self.cfg.phase_len(next) / 60).to_string()],
            );
        } else {
            self.enable_dnd().await;
            self.notify(
                "critical",
                "Break over",
                &format!("{} \u{2022} {} min", next.label(), self.cfg.work_min),
            )
            .await;
            self.sound("alarm-clock-elapsed");
            spawn_detached(
                "focus-break.sh",
                &[next.label(), &(self.cfg.phase_len(next) / 60).to_string()],
            );
        }
    }

    /// Control-socket verbs: `pomo start [task] | toggle | click | reset |
    /// mute [min] | note | resume <text> | idle 0|1 | unlock | pause |
    /// unpause | status`.
    pub async fn control(&mut self, verb: &str, arg: Option<&str>, vars: &mut Vars) -> String {
        let n = now();
        match verb {
            "start" => {
                if self.state != Run::Idle {
                    return "already running".into();
                }
                self.begin_work(n, arg.map(str::to_string), vars).await;
                "ok".into()
            }
            // clock.sh:396-405 `--toggle` — idle -> run (bare, no task) /
            // run <-> pause.
            "toggle" => {
                self.toggle_run_pause(n, vars).await;
                "ok".into()
            }
            // The bar pill's own left-click (genconfig.rs's `pomo_pill`):
            // idle hands off to focus-task.sh (name a task, which then
            // calls `pomo start`) rather than clock.sh's bare start, since
            // FOCUS.md §1.3 wants one named task per work block; running or
            // paused, it's the same pause/resume `toggle` clock.sh's own
            // left-click gave (clock.sh:396-405) — nothing clock.sh offered
            // is lost, naming a task is just interposed on the idle case.
            // T20: `--launch` was missing — focus-task.sh's own guard
            // (`[ "$1" = --launch ] || exit 0`) made a bare spawn exit
            // silently, so the pill's idle left-click did nothing at all.
            "click" => {
                if self.state == Run::Idle {
                    spawn_detached("focus-task.sh", &["--launch"]);
                } else {
                    self.toggle_run_pause(n, vars).await;
                }
                "ok".into()
            }
            "reset" => {
                self.state = Run::Idle;
                self.task = None;
                self.distractions = 0;
                self.nag_first = None;
                self.nag_last = None;
                let _ = self.timer.disarm();
                self.release_dnd().await;
                self.save();
                self.refresh(vars);
                "ok".into()
            }
            // T20: no-op while idle — mute silences a running block's
            // alerts; with no block there is nothing to silence, and the
            // pill hides its mute glyph while idle (see refresh()), so an
            // idle mute would flip invisible state.
            "mute" => {
                if self.state == Run::Idle {
                    return "idle".into();
                }
                if self.muted_at(n) {
                    self.mute_until = None;
                } else {
                    let min: i64 = arg
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(self.cfg.mute_min);
                    self.mute_until = Some(n + min * 60);
                }
                self.refresh(vars);
                "ok".into()
            }
            // FOCUS.md §2.2/§5.4 — focus-note.sh already wrote the parking-
            // lot line itself; this just bumps the tally shown in the popup.
            "note" => {
                self.distractions += 1;
                self.refresh(vars);
                "ok".into()
            }
            "resume" => {
                self.resume = arg.map(str::to_string);
                self.refresh(vars);
                "ok".into()
            }
            "idle" => {
                self.idle_flag = arg == Some("1");
                "ok".into()
            }
            // hypridle's lock listener `on-resume` (hypridle.conf) —
            // replaces clock.sh's polled `pgrep swaylock` lock-flag dance
            // (clock.sh:238-247) with a direct event. Narrower than
            // clock.sh's version: it fires only after an idle-timeout lock,
            // not a manual SUPER+L lock, since only the idle listener has an
            // on-resume hook to wire this to. Only starts while idle, same
            // guard clock.sh's autostart() had, and never while a VNC client
            // is connected: remote input resets that same idle listener, so
            // the autostart would fire for a user who is not at the desk and
            // ring the work bell in an empty room. An explicit `start` or a
            // pill click from the remote session still works.
            "unlock" => {
                if autostart_allowed(self.state, remote::vnc_client_connected()) {
                    self.begin_work(n, None, vars).await;
                    self.notify(
                        "low",
                        "Focus",
                        &format!("{} min \u{2022} started on unlock", self.cfg.work_min),
                    )
                    .await;
                }
                "ok".into()
            }
            // mango-sleep-lock's pre-suspend call. `CLOCK_REALTIME` (see
            // sys.rs's `ClockTimer`) advances across a suspend, so a block
            // left running would surface as a `catch_up` phantom-phase
            // cascade on wake — see the module doc's `catch_up` comment.
            // Unlike `toggle`, only pauses an already-*running* timer and
            // reports whether it did (`"ok"` vs `"noop"`), so a caller that
            // fires this unconditionally before every sleep can tell "I
            // paused it" from "it was already idle/paused" and knows
            // whether `unpause` should undo it again on wake.
            "pause" => {
                if self.state != Run::Running {
                    return "noop".into();
                }
                self.pause_now(n, vars).await;
                "ok".into()
            }
            // The other half of `pause` — resumes a block *this caller*
            // paused. No-op if the user had already paused it by hand (or
            // nothing is running), so it is safe to call unconditionally
            // rather than tracking whether the matching `pause` fired.
            "unpause" => {
                if self.state != Run::Pause {
                    return "noop".into();
                }
                self.resume_now(n, vars).await;
                "ok".into()
            }
            "status" => self.status_line(n),
            _ => "unknown pomo verb".into(),
        }
    }

    /// Idle -> run (bare, no task) / run <-> pause — the body `toggle` and
    /// `click`'s non-idle branch share.
    async fn toggle_run_pause(&mut self, n: i64, vars: &mut Vars) {
        match self.state {
            Run::Idle => self.begin_work(n, None, vars).await,
            Run::Running => self.pause_now(n, vars).await,
            Run::Pause => self.resume_now(n, vars).await,
        }
    }

    /// Both meanings of `until` are bounded by the current phase length
    /// (`Running`: a deadline no further out than one phase; `Pause`:
    /// seconds left, never negative and never longer than the phase).
    /// Clamping here rather than at each display site keeps every
    /// downstream reader — bar text, tooltip meter, `resume_now`, the TSV
    /// log — reading one already-valid value.
    fn clamp_until(&mut self, n: i64) {
        let len = self.cfg.phase_len(self.phase);
        match self.state {
            Run::Idle => {}
            // High end only. A deadline already in the past is legitimate
            // (daemon was down) and `catch_up`'s `while self.until <= n`
            // loop must still see it to roll through every missed boundary.
            Run::Running => self.until = self.until.min(n + len),
            Run::Pause => self.until = self.until.clamp(0, len),
        }
    }

    /// Running -> Pause. Split out of `toggle_run_pause` so the `pause`
    /// control verb (mango-sleep-lock's pre-suspend call) can reach the same
    /// state change directly, guarded by its own `state == Run::Running`
    /// check in `control()` rather than `toggle`'s blind flip.
    async fn pause_now(&mut self, n: i64, vars: &mut Vars) {
        self.until -= n; // becomes "seconds left"
        self.state = Run::Pause;
        // A clock step or a late pause can leave `until` out of range.
        self.clamp_until(n);
        // FOCUS.md §5.3: DND covers an active work block, not a paused
        // one — stepping away mid-block should not also silence everything
        // else indefinitely.
        if self.phase == Phase::Work {
            self.release_dnd().await;
        }
        self.save();
        self.refresh(vars);
    }

    /// Pause -> Running. The `unpause` half of `pause_now`, see its doc
    /// comment.
    async fn resume_now(&mut self, n: i64, vars: &mut Vars) {
        self.until += n; // becomes a deadline again
        self.state = Run::Running;
        let _ = self.timer.arm_at(self.until);
        if self.phase == Phase::Work {
            self.enable_dnd().await;
        }
        self.save();
        self.refresh(vars);
    }

    async fn begin_work(&mut self, n: i64, task: Option<String>, vars: &mut Vars) {
        self.state = Run::Running;
        self.phase = Phase::Work;
        self.phase_start = n;
        self.until = n + self.cfg.phase_len(Phase::Work);
        self.task = task;
        self.distractions = 0;
        self.resume = None;
        self.nag_first = None;
        self.nag_last = None;
        let _ = self.timer.arm_at(self.until);
        self.enable_dnd().await;
        self.save();
        self.refresh(vars);
    }

    fn status_line(&self, n: i64) -> String {
        format!(
            "{:?} {:?} until={} count={} task={} distractions={}",
            self.state,
            self.phase,
            self.until,
            self.count,
            self.task.as_deref().unwrap_or(""),
            self.distractions,
        ) + &format!(" now={n}")
    }

    // -------------------------------------------------------------- alerts

    /// Detached (`sound()`'s own reasoning applies just as much here): a
    /// wedged mako used to block the whole event loop on every break nag —
    /// once a minute during a break — since this awaited `notify-send`
    /// inline.
    async fn notify(&self, urgency: &str, title: &str, body: &str) {
        if self.muted_at(now()) {
            return;
        }
        let urgency = urgency.to_string();
        let title = title.to_string();
        let body = body.to_string();
        tokio::spawn(async move {
            run(
                "notify-send",
                &[
                    "-a",
                    "pomodoro",
                    "-u",
                    &urgency,
                    "-h",
                    "string:x-canonical-private-synchronous:pomodoro",
                    &title,
                    &body,
                ],
            )
            .await;
        });
    }

    /// One of three sounds per family, picked by month index — FOCUS.md
    /// §4.4's habituation reset: a fixed tone worn smooth by repetition is
    /// what makes a bell easy to tune out, so the tone itself changes every
    /// month with no extra machinery beyond an index into a short list.
    ///
    /// Spawned detached, never awaited to completion: clock.sh's own
    /// `beep()` backgrounds `paplay` for exactly this reason
    /// (clock.sh:198-202, `(paplay ... &)`) — playback takes a second or
    /// more, and awaiting it here would stall every other pill's IPC flush
    /// until the sound finished.
    fn sound(&self, base: &str) {
        if self.muted_at(now()) {
            return;
        }
        let family: &[&str] = match base {
            "complete" => &["complete", "bell", "message-new-instant"],
            "alarm-clock-elapsed" => &[
                "alarm-clock-elapsed",
                "dialog-information",
                "phone-incoming-call",
            ],
            _ => &[base],
        };
        let month = usize::try_from((now() / 2_629_800) % family.len() as i64).unwrap_or(0);
        let path = format!("/usr/share/sounds/freedesktop/stereo/{}.oga", family[month]);
        if std::path::Path::new(&path).exists() {
            tokio::spawn(async move {
                let _ = Command::new("paplay").arg(&path).output().await;
            });
        }
    }

    /// Detached, same reasoning as `notify()` above — a wedged mako must not
    /// block the event loop on every work-block start.
    async fn enable_dnd(&self) {
        tokio::spawn(async move {
            run("makoctl", &["mode", "-a", "do-not-disturb"]).await;
        });
    }

    /// `pub(crate)`, not private: main.rs's SIGTERM handler calls this
    /// directly so a killed daemon never leaves DND stuck on. Kept as a
    /// bounded *await* rather than detached like `enable_dnd` — detaching it
    /// would let the process exit before `makoctl` ever ran; `run()`'s own
    /// timeout is what keeps it from hanging the shutdown path instead.
    pub(crate) async fn release_dnd(&self) {
        run("makoctl", &["mode", "-r", "do-not-disturb"]).await;
    }

    // ------------------------------------------------------------- refresh

    /// Same name as every other collector's own `refresh()` (clock.rs,
    /// darkmode.rs, ...) — public so main.rs can render the loaded/restored
    /// state once at startup, before any event has fired.
    pub fn refresh(&self, vars: &mut Vars) {
        let n = now();
        // T20: two-space gap — one space read as the mute bell touching the
        // minutes text. Not rendered at all while idle: mute only silences
        // a running block's alerts (control()'s own idle guard), so an idle
        // pill showing a bell would be dead chrome.
        let muted_glyph = if self.state != Run::Idle && self.muted_at(n) {
            format!("<span alpha='55%'>  {}</span>", tooltip::barico(IC_MUTED))
        } else {
            String::new()
        };
        let (text, class) = match self.state {
            Run::Idle => (tooltip::barico(IC_TIMER), "idle"),
            Run::Running => {
                let left = (self.until - n).max(0);
                let mins = (left + 59) / 60; // ceiling — never reads 0m while still running
                let task = self
                    .task
                    .as_deref()
                    .map(|t| format!(" {t}"))
                    .unwrap_or_default();
                (
                    format!(
                        "{} <span alpha='70%'>{task} {mins}m</span>{muted_glyph}",
                        tooltip::barico_label(IC_TIMER),
                    ),
                    self.phase.css(),
                )
            }
            Run::Pause => {
                let mins = (self.until + 59) / 60;
                (
                    format!(
                        "{} <span alpha='45%'>{mins}m</span>{muted_glyph}",
                        tooltip::barico_label(IC_TIMER),
                    ),
                    "paused",
                )
            }
        };
        vars.set("pomo_text", text);
        vars.set(&format!("{CLASS_PREFIX}pomo"), class);
        tooltip::set_tip(vars, "pomo_tip", self.tip(n));
    }

    fn tip(&self, n: i64) -> String {
        let mut out = String::new();
        out.push_str(&tooltip::sect(&IC_TIMER.to_string(), "Pomodoro"));
        match self.state {
            Run::Running | Run::Pause => {
                let len = self.cfg.phase_len(self.phase);
                let left = if self.state == Run::Running {
                    (self.until - n).max(0)
                } else {
                    self.until
                };
                let done_pct = if len > 0 { (len - left) * 100 / len } else { 0 };
                let paused = if self.state == Run::Pause {
                    " \u{2022} paused"
                } else {
                    ""
                };
                out.push_str(&tooltip::row(&format!(
                    "{}{paused} \u{2022} {} left",
                    self.phase.label(),
                    tooltip::hdur(left),
                )));
                out.push_str(&tooltip::bar(done_pct, tooltip::C_GOOD, 20));
                if let Some(task) = &self.task {
                    out.push_str(&tooltip::dim(&format!("task: {task}")));
                }
                out.push_str(&tooltip::dim(&format!(
                    "{} done this session \u{2022} {} distraction{} this block",
                    self.count,
                    self.distractions,
                    if self.distractions == 1 { "" } else { "s" },
                )));
            }
            // Idle has no body line of its own — the HINTS footer
            // ("click: start/pause · right-click: mute") already says this;
            // a body line repeating it was a duplicate, not a fallback.
            Run::Idle => {}
        }
        if let Some(r) = &self.resume {
            out.push_str(&tooltip::dim(&format!("resume: {r}")));
        }
        if self.muted_at(n) {
            let end = self.mute_until.unwrap_or(n);
            out.push_str(&tooltip::dim(&format!(
                "muted \u{2022} {} left",
                tooltip::hdur(end - n)
            )));
        }
        out
    }
}

/// The `unlock` autostart rule, split out of `control()` so it can be tested
/// without a filesystem: only from `Idle`, and never while a VNC client holds
/// the session (see `remote::vnc_client_connected`).
fn autostart_allowed(state: Run, vnc: bool) -> bool {
    state == Run::Idle && !vnc
}

/// Fires a helper script in `mango/scripts/` detached from the daemon —
/// same shape as `claude::spawn_fetch`'s doc comment: must never be awaited
/// inline, since `focus-break.sh`'s rofi overlay and `focus-resume.sh`'s
/// prompt both block on user interaction, which would freeze every other
/// pill's IPC flush until dismissed.
fn spawn_detached(script: &str, args: &[&str]) {
    let path = format!(
        "{}/.config/mango/scripts/{script}",
        std::env::var("HOME").unwrap_or_default()
    );
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::spawn(async move {
        let _ = Command::new(path).args(&args).output().await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            work_min: 25,
            short_min: 5,
            long_min: 15,
            cycle: 4,
            mute_min: 30,
        }
    }

    #[test]
    fn phase_len_matches_clock_sh() {
        let c = cfg();
        assert_eq!(c.phase_len(Phase::Work), 1500);
        assert_eq!(c.phase_len(Phase::Short), 300);
        assert_eq!(c.phase_len(Phase::Long), 900);
    }

    // clock.sh:273-276
    #[test]
    fn next_phase_cycles_short_short_short_long() {
        let c = cfg();
        assert_eq!(c.next_phase(Phase::Work, 1), Phase::Short);
        assert_eq!(c.next_phase(Phase::Work, 3), Phase::Short);
        assert_eq!(c.next_phase(Phase::Work, 4), Phase::Long);
        assert_eq!(c.next_phase(Phase::Work, 8), Phase::Long);
    }

    // clock.sh:277
    #[test]
    fn break_always_returns_to_work() {
        let c = cfg();
        assert_eq!(c.next_phase(Phase::Short, 3), Phase::Work);
        assert_eq!(c.next_phase(Phase::Long, 4), Phase::Work);
    }

    // The VNC guard: hypridle's on-resume fires for remote input too, so an
    // autostart while a client is connected would ring in an empty room.
    #[test]
    fn unlock_autostarts_only_when_idle_and_no_vnc_client() {
        assert!(autostart_allowed(Run::Idle, false));
        assert!(!autostart_allowed(Run::Idle, true));
        assert!(!autostart_allowed(Run::Running, false));
        assert!(!autostart_allowed(Run::Pause, false));
    }

    fn pomo() -> Pomo {
        Pomo {
            timer: ClockTimer::new().unwrap(),
            cfg: cfg(),
            state: Run::Idle,
            phase: Phase::Work,
            until: 0,
            phase_start: 0,
            count: 0,
            task: None,
            distractions: 0,
            resume: None,
            mute_until: None,
            nag_first: None,
            nag_last: None,
            idle_flag: false,
        }
    }

    // clock.sh:322-329 — a work phase whose deadline passed rolls into a
    // break and banks the count.
    #[tokio::test]
    async fn catch_up_rolls_work_into_short_break_and_banks_count() {
        let mut p = pomo();
        p.state = Run::Running;
        p.phase = Phase::Work;
        p.phase_start = 700;
        p.until = 1000;
        p.catch_up(1000, false).await;
        assert_eq!(p.phase, Phase::Short);
        assert_eq!(p.count, 1);
        assert_eq!(p.until, 1000 + 300);
    }

    // clock.sh:330-332 — the 4th work block rolls into a long break.
    #[tokio::test]
    async fn catch_up_rolls_fourth_work_block_into_long_break() {
        let mut p = pomo();
        p.state = Run::Running;
        p.phase = Phase::Work;
        p.phase_start = 700;
        p.until = 1000;
        p.count = 3;
        p.catch_up(1000, false).await;
        assert_eq!(p.phase, Phase::Long);
        assert_eq!(p.count, 4);
    }

    // clock.sh:333-335 — must not roll over before the deadline.
    #[tokio::test]
    async fn catch_up_does_nothing_before_the_deadline() {
        let mut p = pomo();
        p.state = Run::Running;
        p.phase = Phase::Work;
        p.until = 2000;
        p.catch_up(1000, false).await;
        assert_eq!(p.phase, Phase::Work);
        assert_eq!(p.count, 0);
    }

    // A daemon restart after a long gap must step through every boundary
    // it slept through, not just the first — the behavior clock.sh's own
    // "advance across boundaries" comment describes but its single `if`
    // only delivered by being re-polled every second.
    #[tokio::test]
    async fn catch_up_steps_through_multiple_missed_boundaries() {
        let mut p = pomo();
        p.state = Run::Running;
        p.phase = Phase::Work;
        p.phase_start = 0;
        p.until = 1500; // work ends at 1500
                        // Jump far past the short break (1500+300=1800) into the next work
                        // block, as if the daemon was down the whole time.
        p.catch_up(1900, false).await;
        assert_eq!(p.phase, Phase::Work);
        assert_eq!(p.count, 1);
        assert_eq!(p.until, 1800 + 1500);
    }

    // clock.sh:340-347 — muted() straddles its deadline exactly at the
    // current second.
    #[test]
    fn muted_straddles_its_deadline() {
        let mut p = pomo();
        p.mute_until = Some(1005);
        assert!(p.muted_at(1000));
        p.mute_until = Some(999);
        assert!(!p.muted_at(1000));
        p.mute_until = None;
        assert!(!p.muted_at(1000));
    }

    // clock.sh:364-385 — nag(): quiet in the grace window and while idle,
    // warns once past it, respects the repeat spacing, escalates to a
    // sound only past NAG_ESCALATE.
    #[tokio::test]
    async fn nag_grace_repeat_and_escalation() {
        let mut p = pomo();
        p.state = Run::Running;
        p.phase = Phase::Short;
        p.phase_start = 90;

        p.nag(140).await; // 50s in, past the 60s grace? no — under it
        assert!(
            p.nag_first.is_none(),
            "must stay quiet inside the grace window"
        );

        p.idle_flag = true;
        p.nag(200).await; // past grace, but idle
        assert!(p.nag_first.is_none(), "must stay quiet while idle");
        p.idle_flag = false;

        p.nag(160).await; // 70s in, past the 60s grace
        assert_eq!(
            p.nag_first,
            Some(160),
            "must warn once past the grace window"
        );

        p.nag(180).await; // 20s after the first warning
        assert_eq!(p.nag_last, Some(160), "must not repeat inside NAG_EVERY");

        p.nag(221).await; // 61s after the first warning
        assert_eq!(p.nag_last, Some(221), "must repeat after NAG_EVERY");

        p.nag(460).await; // 300s after the first warning: escalates
        assert_eq!(p.nag_last, Some(460));
    }

    // `pause` must move a running block into `Pause` and report "ok" — the
    // sleep-lock daemon relies on this reply to know it must `unpause` on
    // wake.
    #[tokio::test]
    async fn pause_stops_a_running_block() {
        let mut p = pomo();
        let mut vars = Vars::new();
        p.state = Run::Running;
        p.phase = Phase::Work;
        p.until = 2000;
        let reply = p.control("pause", None, &mut vars).await;
        assert_eq!(reply, "ok");
        assert_eq!(p.state, Run::Pause);
    }

    // A pause taken after the deadline already passed (daemon lagging the
    // timerfd boundary, or a CLOCK_REALTIME step across suspend) must not
    // store a negative remaining — the pill would render "-2m" forever.
    #[tokio::test]
    async fn pause_after_deadline_clamps_remaining_to_zero() {
        let mut p = pomo();
        let mut vars = Vars::new();
        p.state = Run::Running;
        p.phase = Phase::Work;
        p.until = 1000;
        p.pause_now(1200, &mut vars).await; // 200s past the deadline
        assert_eq!(p.until, 0, "remaining must clamp to zero, not go negative");
    }

    // A `Pause` state's stored remaining is bounded by its own phase length
    // no matter how it got there (a stale/torn state file, a shorter
    // MANGO_POMODORO reload) — `load()` calls this same clamp on every read
    // rather than trusting the file. Exercised on `clamp_until` directly,
    // not through `load()` itself: `load()`/`save()` share
    // `$XDG_RUNTIME_DIR/mango-bard-pomo` with the real running daemon, and a
    // test writing that file would race it.
    #[test]
    fn load_clamps_remaining_longer_than_the_phase() {
        let mut p = pomo();
        p.state = Run::Pause;
        p.phase = Phase::Short; // phase_len = 300
        p.until = 99_999; // absurdly long remaining
        p.clamp_until(now());
        assert_eq!(
            p.until, 300,
            "remaining must not exceed its own phase length"
        );
    }

    // Idle and already-paused must both report "noop" and leave state
    // untouched — this is how the caller tells "I paused it" from "there
    // was nothing to pause", so it knows whether to `unpause` on wake.
    #[tokio::test]
    async fn pause_is_a_noop_while_idle_or_already_paused() {
        let mut p = pomo();
        let mut vars = Vars::new();
        assert_eq!(p.control("pause", None, &mut vars).await, "noop");
        assert_eq!(p.state, Run::Idle);

        p.state = Run::Pause;
        p.until = 300;
        assert_eq!(p.control("pause", None, &mut vars).await, "noop");
        assert_eq!(p.state, Run::Pause);
        assert_eq!(p.until, 300, "must not touch the stored remaining time");
    }

    // `unpause` must move a paused block back to `Running` and report "ok".
    #[tokio::test]
    async fn unpause_resumes_a_paused_block() {
        let mut p = pomo();
        let mut vars = Vars::new();
        p.state = Run::Pause;
        p.phase = Phase::Work;
        p.until = 300; // seconds left, per the Pause-state convention
        let reply = p.control("unpause", None, &mut vars).await;
        assert_eq!(reply, "ok");
        assert_eq!(p.state, Run::Running);
    }

    // Idle and already-running must both report "noop" — a manually-paused
    // block that the sleep-lock daemon never touched must not be resumed by
    // a stray `unpause` call, and an idle daemon has nothing to resume.
    #[tokio::test]
    async fn unpause_is_a_noop_while_idle_or_already_running() {
        let mut p = pomo();
        let mut vars = Vars::new();
        assert_eq!(p.control("unpause", None, &mut vars).await, "noop");
        assert_eq!(p.state, Run::Idle);

        p.state = Run::Running;
        p.until = 2000;
        assert_eq!(p.control("unpause", None, &mut vars).await, "noop");
        assert_eq!(p.state, Run::Running);
        assert_eq!(p.until, 2000, "must not touch the deadline");
    }
}
