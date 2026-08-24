//! The power collector: battery pill plus its wear/power popup, and the
//! AC-edge trigger for `powermode.sh`. Ports src/waybar/scripts/battery.sh
//! (`custom/battery`, a 15s poll) and src/mango/scripts/ac-watch.sh (a
//! separate `udevadm monitor` watcher) — see IRONBAR.md T5 for the design
//! and its two corrections to the plan as originally written:
//!
//! 1. `battery-guard.sh` is **not** absorbed. It is a 370-line power-policy
//!    engine (weak-charger latch, eco escalation, docker/ollama drain,
//!    suspend ladder) whose 30s poll cadence is load-bearing —
//!    `powermode.conf`'s `PM_WEAK_POLLS` is defined in terms of it — and
//!    whose separateness is exactly IRONBAR.md goal 5's sanctioned "one
//!    trust boundary": the daemon reports state and triggers `powermode.sh`,
//!    it never owns power policy itself. It keeps running unmodified.
//! 2. `ac-watch.sh` **is** absorbed, fully: pill, `powermode.sh` trigger,
//!    notification and chime all ride the one udev stream this collector
//!    already needs for the pill, removing a second `udevadm monitor` child.
//!
//! No zbus/UPower (same reasoning as T3's net.rs): wear, energy and rate are
//! all derivable from sysfs — `charge_full / charge_full_design` matches
//! `upower -i`'s own `capacity:` field exactly (verified on this machine:
//! 2954000/3550000 = 83.2%).
//!
//! `refresh()` is the only thing that touches sysfs/upower/RAPL; it forks
//! only for the detached AC-edge side effects (`notify-send`, `paplay`,
//! `powermode.sh`) and the `udevadm monitor` child that wakes it. The
//! previous RAPL sample lives in `self` rather than a state file
//! (battery.sh needed a file because it re-execs every poll; this daemon
//! does not), and there is no tooltip cache — the 60s cache existed only to
//! survive a 15s poll interval that no longer exists once refreshes are
//! purely event-driven. Consequence accepted: "where it goes" watts become
//! an average over the irregular gap since the last refresh, not over 15s.

use crate::mango::CLASS_PREFIX;
use crate::net::MonitorChild;
use crate::powermode;
use crate::tooltip::{bad, bar, barico_label, esc, grade, hdur, kv, kvsub, mono, set_titled, C_GOOD};
use crate::vars::Vars;
use std::path::{Path, PathBuf};
use tokio::process::Command;

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

// ------------------------------------------------------------------ icons
//
// Material Symbols Rounded codepoints, copied verbatim from battery.sh's own
// printf byte sequences (battery.sh:46,66 — decoded and named here instead,
// same idiom as net.rs's icon table).

/// battery_charging_full, was U+E1A3 (Material Symbols).
/// T8b: -> U+F1E6 (plug, JetBrainsMono Nerd Font Font Awesome) — GTK4
/// cannot correctly rasterize Material Symbols Rounded's variable font on
/// this system; see IRONBAR.md's T8b entry. The first two replacements
/// tried (U+F0E7 and U+F427, both bolt/lightning glyphs) rendered as wrong
/// CJK tofu live in ironbar despite being genuine Nerd Font codepoints —
/// only U+F1E6 was confirmed working by an actual live-ironbar screenshot.
/// T19: U+F1E6 -> U+F06A5 (md-power_plug) — one-icon-family sweep
/// (IRONBAR.md T19); it no longer needs to also stand in for net.rs's
/// `IC_ETH`, which kept its own distinct md-ethernet glyph throughout.
const IC_CHG: char = '\u{f06a5}';
/// energy_savings_leaf, was U+EC1A — the eco-mode marker on the bar icon.
/// T8b: -> U+F1BB (mountain/tree, Nerd Font Font Awesome) — U+F06C (a
/// literal leaf) rendered as wrong CJK tofu live in ironbar; U+F1BB was
/// confirmed working by an actual live-ironbar screenshot. See IC_CHG's
/// comment above and IRONBAR.md's T8b entry for why a Nerd Font source
/// alone doesn't guarantee a correct GTK4 render.
/// T19: U+F1BB -> U+F032A (md-leaf, an actual leaf rather than a mountain)
/// — one-icon-family sweep (IRONBAR.md T19).
const IC_LEAF: char = '\u{f032a}';

/// battery.sh:47-57 — battery_0_bar..battery_6_bar are not contiguous
/// codepoints, hence the table rather than an offset.
///
/// T8b: the seven Material Symbols "battery_N_bar" glyphs this used to
/// return all render as wrong CJK substitutes under GTK4 on this system
/// (see IRONBAR.md's T8b entry). JetBrainsMono Nerd Font's Font Awesome
/// range had only five distinct battery-level glyphs, so two adjacent
/// buckets shared a glyph (1/2 and 5/6) — the actual `pct` was always shown
/// as text next to the icon, so that lost only the icon's own resolution,
/// not the underlying data.
///
/// T19: full resolution restored — `nf-md` has a complete seven-step
/// battery ramp, so each of the seven `(pct+8)/17` buckets now gets its own
/// glyph. Part of the one-icon-family sweep (IRONBAR.md T19).
pub fn ic_bat(pct: i64) -> char {
    match (pct + 8) / 17 {
        0 => '\u{f008e}', // md-battery_outline
        1 => '\u{f007a}', // md-battery_10
        2 => '\u{f007c}', // md-battery_30
        3 => '\u{f007e}', // md-battery_50
        4 => '\u{f0080}', // md-battery_70
        5 => '\u{f0082}', // md-battery_90
        _ => '\u{f0079}', // md-battery (full)
    }
}

/// battery.sh:393-396 — `Charging`/`Not charging` show the plug icon even
/// though `Not charging` keeps the `discharging`/`critical` *class*
/// ([`class_for`]) — ACPI can report "Not charging" while sitting at a
/// charge ceiling with the cable in, which reads as plugged, not draining.
pub fn icon_for(status: &str, charge: i64) -> char {
    match status {
        "Charging" | "Not charging" => IC_CHG,
        _ => ic_bat(charge),
    }
}

/// battery.sh:398-400 — `critical` at <=20% overrides `full`/`discharging`
/// but never `charging`.
pub fn class_for(status: &str, charge: i64) -> &'static str {
    let mut class = match status {
        "Charging" => "charging",
        "Full" => "full",
        _ => "discharging",
    };
    if charge <= 20 && class != "charging" {
        class = "critical";
    }
    class
}

// ---------------------------------------------------------------- levels

pub struct Levels {
    pub now: i64,
    pub full: i64,
    pub design: i64,
    pub rate: i64,
    pub unit: &'static str,
}

/// The eight raw sysfs attributes [`levels`] chooses between — kept separate
/// from the read itself so the choice (charge-type vs energy-type, design
/// falling back to full) is a pure, fixture-testable function.
#[derive(Default)]
pub struct RawBat {
    pub charge_now: Option<i64>,
    pub charge_full: Option<i64>,
    pub charge_full_design: Option<i64>,
    pub current_now: Option<i64>,
    pub energy_now: Option<i64>,
    pub energy_full: Option<i64>,
    pub energy_full_design: Option<i64>,
    pub power_now: Option<i64>,
}

/// battery.sh:78-88 — charge-type battery preferred, energy-type fallback;
/// `design` falls back to `full` when absent (not every battery exposes a
/// design-capacity node even when `*_now`/`*_full` exist).
pub fn levels(raw: &RawBat) -> Option<Levels> {
    if let (Some(n), Some(f)) = (raw.charge_now, raw.charge_full) {
        return Some(Levels {
            now: n,
            full: f,
            design: raw.charge_full_design.unwrap_or(f),
            rate: raw.current_now.unwrap_or(0),
            unit: "Ah",
        });
    }
    if let (Some(n), Some(f)) = (raw.energy_now, raw.energy_full) {
        return Some(Levels {
            now: n,
            full: f,
            design: raw.energy_full_design.unwrap_or(f),
            rate: raw.power_now.unwrap_or(0),
            unit: "Wh",
        });
    }
    None
}

/// battery.sh:90 — round-half-up on a float, truncated by the printf `%d`.
pub fn pct(a: i64, b: i64) -> i64 {
    if b > 0 {
        (a as f64 * 100.0 / b as f64 + 0.5) as i64
    } else {
        0
    }
}

/// battery.sh:93 — micro-units to "1.25 Ah".
pub fn uh(v: i64, unit: &str) -> String {
    format!("{:.2} {unit}", v as f64 / 1_000_000.0)
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// battery.sh:96-100 — uA x uV -> W, or uW -> W when the battery already
/// reports energy.
pub fn watts_num(rate: i64, voltage: i64, unit: &str) -> f64 {
    let w = if unit == "Ah" {
        rate as f64 * voltage as f64 / 1e12
    } else {
        rate as f64 / 1e6
    };
    round1(w)
}

pub fn watts(rate: i64, voltage: i64, unit: &str) -> String {
    format!("{:.1} W", watts_num(rate, voltage, unit))
}

/// battery.sh:224-229 — seconds until empty (discharging) or full
/// (charging); `-1` when the rate is unknown (reads as zero right after a
/// transition).
pub fn remaining(now: i64, full: i64, rate: i64, status: &str) -> i64 {
    if rate <= 0 {
        return -1;
    }
    let numerator = if status == "Charging" {
        full - now
    } else {
        now
    };
    (numerator as f64 / rate as f64 * 3600.0) as i64
}

// ------------------------------------------------------------------- RAPL

/// battery.sh:112-120 — `energy_uj` deltas wrap around
/// `max_energy_range_uj` rather than going negative forever; a wrap must
/// read as a small positive draw, never negative or huge nonsense wattage.
pub fn rapl_watts(prev_uj: i64, prev_ns: i64, cur_uj: i64, cur_ns: i64, max_range_uj: i64) -> f64 {
    let mut d = cur_uj - prev_uj;
    if d < 0 {
        d = if max_range_uj > 0 {
            d + max_range_uj
        } else {
            0
        };
    }
    if d < 0 {
        d = 0;
    }
    let dt = (cur_ns - prev_ns) as f64 / 1_000_000_000.0;
    if dt > 0.0 {
        round1(d as f64 / dt / 1_000_000.0)
    } else {
        0.0
    }
}

/// battery.sh:123-128 — a part's share of a whole, clamped to 0..=100 for
/// [`bar`].
pub fn sharepct(part: f64, whole: f64) -> i64 {
    let v = if whole > 0.0 {
        part * 100.0 / whole
    } else {
        0.0
    };
    v.clamp(0.0, 100.0).round() as i64
}

// ------------------------------------------------------------------- wear

/// battery.sh:146-156 — projects days until health decays to `replace`, from
/// this battery's own cycle-count history. Divides by *total* cycles (wear
/// per cycle integrates the battery's whole life), not the window's cycle
/// delta — only the cycles-per-day term uses the window. Sentinels:
/// `-2` health already at/under `replace` ("replace now"), `-1` under 7
/// days of span or no new cycle in it ("learning"), `-3` rate computed as
/// <=0 e.g. an EC recalibration bumped health back up (omit the clause).
pub fn wear_days(
    health: i64,
    cycles: i64,
    hist_epoch: i64,
    hist_cycles: i64,
    now: i64,
    replace: i64,
) -> i64 {
    if health <= replace {
        return -2;
    }
    let span = (now - hist_epoch) as f64 / 86400.0;
    let dcyc = cycles - hist_cycles;
    if span < 7.0 || dcyc <= 0 {
        return -1;
    }
    let rate = ((100 - health) as f64 / cycles as f64) * (dcyc as f64 / span);
    if rate <= 0.0 {
        return -3;
    }
    ((health - replace) as f64 / rate) as i64
}

/// battery.sh:160-169 — hdur's pair for multi-month spans (hdur caps at
/// hours, which would print "3000h 00m" for a year-out projection).
pub fn ddur(days: i64) -> String {
    if days >= 365 {
        let y = days / 365;
        let m = (days % 365) / 30;
        if m > 0 {
            format!("~{y}y {m}mo")
        } else {
            format!("~{y}y")
        }
    } else if days >= 30 {
        format!("~{}mo", days / 30)
    } else {
        format!("~{days}d")
    }
}

pub struct WearRow {
    pub epoch: i64,
    pub cycles: i64,
    pub full: i64,
}

/// battery-wear.tsv line format: "epoch\tcycle_count\tcharge_full".
/// `full` is written but never read back (kept for forward compat, matching
/// the shell's own comment on why it writes a column it doesn't use).
pub fn parse_wear_row(line: &str) -> Option<WearRow> {
    let mut it = line.split('\t');
    Some(WearRow {
        epoch: it.next()?.parse().ok()?,
        cycles: it.next()?.parse().ok()?,
        full: it.next()?.parse().ok()?,
    })
}

/// battery.sh:481 — keep rows still inside the trailing window.
pub fn wear_survives_prune(row: &WearRow, now: i64, window: i64) -> bool {
    row.epoch >= now - window
}

// ------------------------------------------------------------- power_stats

pub struct PowerSample {
    pub epoch: i64,
    pub watts: f64,
    pub state: String,
}

pub struct PowerStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    /// Oldest-to-newest, matching battery.sh's own printed order (heatbar
    /// consumes it directly in that order).
    pub buckets: Vec<i64>,
}

/// battery.sh:187-210 — only `discharging` samples inside the window count
/// (on AC the upower history file still fills with `charging`/`unknown`
/// rows). Empty buckets carry the previous (older) bucket's value forward
/// rather than reading as 0W, or a quiet stretch looks like an idle trough
/// that never happened; the very first bucket with nothing older to inherit
/// from seeds at 50 (mid-scale, not a spike or a trough).
pub fn power_stats(
    samples: &[PowerSample],
    now: i64,
    window: i64,
    buckets: usize,
) -> Option<PowerStats> {
    let mut bs = vec![0.0f64; buckets];
    let mut bn = vec![0i64; buckets];
    let (mut n, mut sum, mut mn, mut mx) = (0i64, 0.0f64, 0.0f64, 0.0f64);
    for s in samples {
        if s.state != "discharging" || s.epoch < now - window {
            continue;
        }
        n += 1;
        sum += s.watts;
        if n == 1 || s.watts < mn {
            mn = s.watts;
        }
        if n == 1 || s.watts > mx {
            mx = s.watts;
        }
        let mut b = ((now - s.epoch) * buckets as i64) / window;
        if b >= buckets as i64 {
            b = buckets as i64 - 1;
        }
        if b < 0 {
            b = 0;
        }
        let b = b as usize;
        bs[b] += s.watts;
        bn[b] += 1;
    }
    if n == 0 {
        return None;
    }
    let span = mx - mn;
    let mut last = 50.0;
    let mut out = Vec::with_capacity(buckets);
    for i in (0..buckets).rev() {
        if bn[i] > 0 {
            let v = bs[i] / bn[i] as f64;
            last = if span > 0.0 {
                (v - mn) * 100.0 / span
            } else {
                50.0
            };
        }
        out.push(last.round() as i64);
    }
    Some(PowerStats {
        min: mn,
        max: mx,
        avg: sum / n as f64,
        buckets: out,
    })
}

/// battery.sh:175-179 — matches
/// `history-rate-<model with spaces->underscores>-*.dat`, picking the
/// battery's history file out of every power-supply device upower also
/// logs (mouse, keyboard, ...).
pub fn matches_hist_file(filename: &str, model: &str) -> bool {
    let prefix = format!("history-rate-{}-", model.replace(' ', "_"));
    filename.starts_with(&prefix) && filename.ends_with(".dat")
}

// --------------------------------------------------------------- ac-watch

/// ac-watch.sh:81-83 — an edge, not an event. The udev stream also carries
/// its own two-line startup banner and a `change` for BAT0 on every
/// capacity tick; reading `online` back and comparing absorbs all of that
/// without parsing anything.
pub fn ac_changed(prev: Option<u8>, now: Option<u8>) -> bool {
    match now {
        None => false,
        Some(n) => prev != Some(n),
    }
}

/// ac-watch.sh:62-67 — driven by `online`, never `$BAT/status`: ACPI lags
/// the transition by a beat, so status can still read Discharging for a
/// moment after the plug goes in.
pub fn ac_summary(online: u8) -> &'static str {
    if online == 1 {
        "Charger connected"
    } else {
        "On battery"
    }
}

/// ac-watch.sh:69-75 — empty body on an unreadable capacity.
pub fn ac_body(online: u8, capacity: Option<i64>) -> String {
    match capacity {
        None => String::new(),
        Some(p) => {
            if online == 1 {
                format!("{p}% · charging")
            } else {
                format!("{p}% · discharging")
            }
        }
    }
}

// ---------------------------------------------------------------- sysfs

fn readf(dir: &Path, attr: &str) -> Option<String> {
    let s = std::fs::read_to_string(dir.join(attr)).ok()?;
    let s = s.trim_end();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn readf_i64(dir: &Path, attr: &str) -> Option<i64> {
    readf(dir, attr)?.parse().ok()
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var(var)
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// battery.sh:370-371 / ac-watch.sh:116-117 — first lexical match of a glob,
/// same discovery idiom shared by battery.sh/ac-watch.sh/battery-guard.sh so
/// `MANGO_BAT_DIR`/`MANGO_AC_DIR` override all three identically.
fn discover(prefixes: &[&str]) -> Option<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| prefixes.iter().any(|p| n.starts_with(p)))
        .collect();
    names.sort();
    names
        .into_iter()
        .next()
        .map(|n| PathBuf::from("/sys/class/power_supply").join(n))
}

fn read_online(ac: &Path) -> Option<u8> {
    readf(ac, "online")?.trim().parse().ok()
}

fn xdg_runtime_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn is_weak_latched() -> bool {
    xdg_runtime_dir().join("mango-powermode.weak").exists()
}

fn is_manual() -> bool {
    xdg_runtime_dir().join("mango-powermode.manual").exists()
}

const WEAR_WINDOW: i64 = 90 * 86400;
const WEAR_APPEND_MIN: i64 = 43_200;

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ Power

struct RaplSample {
    ns: i64,
    pkg_uj: i64,
    unc_uj: i64,
}

pub struct Power {
    pub udev_mon: MonitorChild,
    bat_dir: Option<PathBuf>,
    ac_dir: Option<PathBuf>,
    rapl_pkg: PathBuf,
    rapl_unc: PathBuf,
    upower_dir: PathBuf,
    backlight_dir: Option<PathBuf>,
    wear_state: PathBuf,
    wear_replace: i64,
    last_online: Option<u8>,
    last_rapl: Option<RaplSample>,
    tick: u32,
    discharging: bool,
}

impl Power {
    pub fn new() -> Self {
        let state_home = std::env::var("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
            });
        Self {
            udev_mon: MonitorChild::new(
                "udevadm",
                &["monitor", "--udev", "--subsystem-match=power_supply"],
            ),
            bat_dir: env_dir("MANGO_BAT_DIR").or_else(|| discover(&["BAT"])),
            ac_dir: env_dir("MANGO_AC_DIR").or_else(|| discover(&["AC", "AD"])),
            rapl_pkg: env_dir("MANGO_RAPL_PKG_DIR")
                .unwrap_or_else(|| PathBuf::from("/sys/class/powercap/intel-rapl:0")),
            rapl_unc: env_dir("MANGO_RAPL_UNC_DIR")
                .unwrap_or_else(|| PathBuf::from("/sys/class/powercap/intel-rapl:0:1")),
            upower_dir: env_dir("MANGO_UPOWER_DIR")
                .unwrap_or_else(|| PathBuf::from("/var/lib/upower")),
            backlight_dir: env_dir("MANGO_BACKLIGHT_DIR").or_else(|| {
                std::fs::read_dir("/sys/class/backlight")
                    .ok()
                    .and_then(|mut d| d.find_map(|e| e.ok()).map(|e| e.path()))
            }),
            wear_state: env_dir("MANGO_WEAR_STATE")
                .unwrap_or_else(|| state_home.join("mango/battery-wear.tsv")),
            wear_replace: std::env::var("MANGO_WEAR_REPLACE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(80),
            last_online: None,
            last_rapl: None,
            tick: 0,
            discharging: false,
        }
    }

    /// Called once at startup, before the first [`refresh`](Self::refresh) —
    /// primes `last_online` from a raw read so the first refresh's own
    /// (near-identical) read never looks like an edge. Deliberately no
    /// notification: a daemon restart (`Restart=on-failure`) should not pop
    /// a spurious "Charger connected" toast.
    pub fn prime(&mut self) {
        self.last_online = self.ac_dir.as_deref().and_then(read_online);
    }

    /// Fires once at daemon startup. `auto`, not `cable` (ac-watch.sh's own
    /// startup trigger) — `cable` unconditionally clears the manual and weak
    /// markers, correct at a fresh login but wrong on every
    /// `Restart=on-failure` bounce, where it would silently overwrite a
    /// deliberate mode choice. `auto` honours those markers and is
    /// identical to `cable` when neither exists yet.
    pub fn trigger_startup_mode(&self) {
        spawn_detached(powermode_script_path(), vec!["auto".to_string()]);
    }

    /// 5-minute discharging backstop (IRONBAR.md risk list: battery uevent
    /// granularity varies per machine). No `pm_mode` gate — the period table
    /// lists battery as event-driven with no period; this exists purely as
    /// a staleness bound, not a poll.
    pub fn should_refresh_on_tick(&mut self) -> bool {
        if !self.discharging {
            self.tick = 0;
            return false;
        }
        self.tick = self.tick.wrapping_add(1);
        self.tick.is_multiple_of(5)
    }

    fn rapl_readable(&self) -> bool {
        self.rapl_pkg.join("energy_uj").is_file() && self.rapl_unc.join("energy_uj").is_file()
    }

    fn backlight_pct(&self) -> Option<i64> {
        let dir = self.backlight_dir.as_deref()?;
        let br = readf_i64(dir, "brightness")?;
        let mx = readf_i64(dir, "max_brightness")?;
        if mx <= 0 {
            return None;
        }
        Some(pct(br, mx))
    }

    fn hist_file(&self, model: &str) -> Option<PathBuf> {
        let mut names: Vec<String> = std::fs::read_dir(&self.upower_dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| matches_hist_file(n, model))
            .collect();
        names.sort();
        names.into_iter().next().map(|n| self.upower_dir.join(n))
    }

    fn read_power_samples(&self, path: &Path) -> Vec<PowerSample> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| {
                let mut it = line.split('\t');
                let epoch = it.next()?.parse().ok()?;
                let watts = it.next()?.parse().ok()?;
                let state = it.next()?.to_string();
                Some(PowerSample {
                    epoch,
                    watts,
                    state,
                })
            })
            .collect()
    }

    /// Appends a `battery-wear.tsv` sample at most once per
    /// [`WEAR_APPEND_MIN`], pruning to [`WEAR_WINDOW`] on the same write —
    /// battery.sh:472-485.
    fn append_and_prune_wear(&self, cycles: i64, full: i64) {
        let now = unix_now();
        let last_epoch = std::fs::read_to_string(&self.wear_state)
            .ok()
            .and_then(|s| s.lines().last().and_then(parse_wear_row).map(|r| r.epoch));
        if let Some(e) = last_epoch {
            if now - e < WEAR_APPEND_MIN {
                return;
            }
        }
        if let Some(parent) = self.wear_state.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let kept: Vec<String> = std::fs::read_to_string(&self.wear_state)
            .unwrap_or_default()
            .lines()
            .filter_map(parse_wear_row)
            .filter(|r| wear_survives_prune(r, now, WEAR_WINDOW))
            .map(|r| format!("{}\t{}\t{}", r.epoch, r.cycles, r.full))
            .collect();
        let mut text = kept.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("{now}\t{cycles}\t{full}\n"));
        let _ = std::fs::write(&self.wear_state, text);
    }

    /// The oldest surviving sample is the window's start — battery.sh:490-492.
    fn oldest_wear_sample(&self) -> Option<WearRow> {
        std::fs::read_to_string(&self.wear_state)
            .ok()
            .and_then(|s| s.lines().next().and_then(parse_wear_row))
    }

    fn trigger_ac_edge(&self, online: u8) {
        let capacity = self
            .bat_dir
            .as_deref()
            .and_then(|d| readf_i64(d, "capacity"));
        spawn_notify(ac_summary(online), &ac_body(online, capacity));
        spawn_beep(if online == 1 {
            "power-plug"
        } else {
            "power-unplug"
        });
        spawn_detached(powermode_script_path(), vec!["cable".to_string()]);
    }

    fn emit_absent(&self, vars: &mut Vars) {
        vars.set("bat_text", "");
        set_titled(vars, "bat_tip", "Battery", "No battery".to_string());
        vars.set(&class_key("battery"), "absent");
    }

    /// Re-derives `bat_text`/`bat_tip`/`@class/battery` from a fresh sysfs
    /// read, and — since this is also where `online` gets its fresh read —
    /// detects and acts on an AC edge. Safe to call from any trigger (tick
    /// backstop, powermode change, control socket): when `online` truly
    /// hasn't moved since the last observation, [`ac_changed`] returns
    /// false and no edge action fires, so there is no feedback loop through
    /// `powermode.sh cable` rewriting the mode file and this collector
    /// waking again on that inotify event.
    pub async fn refresh(&mut self, vars: &mut Vars, pm_mode: powermode::Mode) {
        let Some(bat) = self.bat_dir.clone() else {
            self.emit_absent(vars);
            return;
        };
        let raw = RawBat {
            charge_now: readf_i64(&bat, "charge_now"),
            charge_full: readf_i64(&bat, "charge_full"),
            charge_full_design: readf_i64(&bat, "charge_full_design"),
            current_now: readf_i64(&bat, "current_now"),
            energy_now: readf_i64(&bat, "energy_now"),
            energy_full: readf_i64(&bat, "energy_full"),
            energy_full_design: readf_i64(&bat, "energy_full_design"),
            power_now: readf_i64(&bat, "power_now"),
        };
        let Some(lv) = levels(&raw) else {
            self.emit_absent(vars);
            return;
        };

        let status = readf(&bat, "status").unwrap_or_default();
        let voltage = readf_i64(&bat, "voltage_now");
        let cycles = readf_i64(&bat, "cycle_count");
        let model = readf(&bat, "model_name");
        let vendor = readf(&bat, "manufacturer");
        let tech = readf(&bat, "technology");

        let online = self.ac_dir.as_deref().and_then(read_online);
        if ac_changed(self.last_online, online) {
            if let Some(o) = online {
                self.trigger_ac_edge(o);
            }
        }
        self.last_online = online;

        let charge = readf_i64(&bat, "capacity").unwrap_or_else(|| pct(lv.now, lv.full));
        let health = pct(lv.full, lv.design);
        let icon = icon_for(&status, charge);
        let class = class_for(&status, charge);
        self.discharging = status == "Discharging";

        let weak = is_weak_latched();
        let leaf = if pm_mode == powermode::Mode::Eco {
            format!(
                " <span size=\"115%\" rise=\"-1200\"><span foreground=\"{}\">{IC_LEAF}</span></span>",
                crate::tooltip::C_GOOD
            )
        } else {
            String::new()
        };
        let text = format!("{} {charge}%{leaf}", barico_label(icon));

        let secs = remaining(lv.now, lv.full, lv.rate, &status);
        let total_w = watts_num(lv.rate, voltage.unwrap_or(0), lv.unit);

        // RAPL power attribution — battery.sh:412-436. Only meaningful while
        // discharging; a no-op with no error before system/rapl/install.sh
        // has unlocked energy_uj.
        let mut pkg_w = None;
        let mut unc_w = None;
        let mut resid_w = None;
        let mut backlight = None;
        if self.discharging && self.rapl_readable() {
            let pkg_uj = readf_i64(&self.rapl_pkg, "energy_uj");
            let unc_uj = readf_i64(&self.rapl_unc, "energy_uj");
            if let (Some(pkg_uj), Some(unc_uj)) = (pkg_uj, unc_uj) {
                let ns = now_ns();
                if let Some(prev) = &self.last_rapl {
                    let mrp = readf_i64(&self.rapl_pkg, "max_energy_range_uj").unwrap_or(0);
                    let mru = readf_i64(&self.rapl_unc, "max_energy_range_uj").unwrap_or(0);
                    let pw = rapl_watts(prev.pkg_uj, prev.ns, pkg_uj, ns, mrp);
                    let uw = rapl_watts(prev.unc_uj, prev.ns, unc_uj, ns, mru);
                    pkg_w = Some(pw);
                    unc_w = Some(uw);
                    resid_w = Some((total_w - pw).max(0.0));
                    backlight = self.backlight_pct();
                }
                self.last_rapl = Some(RaplSample { ns, pkg_uj, unc_uj });
            }
        }

        // Wear append/prune + projection — battery.sh:468-502.
        let mut hdet_extra = String::new();
        if let Some(c) = cycles.filter(|&c| c != 0) {
            self.append_and_prune_wear(c, lv.full);
            if let Some(hist) = self.oldest_wear_sample() {
                let now = unix_now();
                let wdays = wear_days(health, c, hist.epoch, hist.cycles, now, self.wear_replace);
                hdet_extra = match wdays {
                    -2 => " · replace now".to_string(),
                    -1 => " · wear rate: learning".to_string(),
                    -3 => String::new(),
                    d => format!(" · {} left", ddur(d)),
                };
            }
        }

        let history_line = model
            .as_deref()
            .and_then(|m| self.hist_file(m))
            .and_then(|hf| {
                let samples = self.read_power_samples(&hf);
                power_stats(&samples, unix_now(), 3600, 24)
            });

        let tip = build_tip(
            charge,
            health,
            &lv,
            &status,
            secs,
            online,
            cycles,
            &hdet_extra,
            vendor.as_deref(),
            tech.as_deref(),
            voltage,
            total_w,
            pkg_w,
            unc_w,
            resid_w,
            backlight,
            history_line,
            weak,
            is_manual(),
            pm_mode,
        );

        let title_text = match model.as_deref() {
            Some(m) => format!("Battery · {m}"),
            None => "Battery".to_string(),
        };

        vars.set("bat_text", text);
        set_titled(vars, "bat_tip", &title_text, tip);
        vars.set(&class_key("battery"), class);
    }
}

impl Default for Power {
    fn default() -> Self {
        Self::new()
    }
}

/// battery.sh:449-546's TIP subshell, straight-line ported: title+rule,
/// Charge, Health+wear, vendor/tech, Power+history, "Where it goes", Mode,
/// click hint. `kv`/`kvsub` self-terminate with `\n`; `dim` (the final
/// line) does not, matching the shell's own trailing-newline-free output
/// once `$()` strips it.
#[allow(clippy::too_many_arguments)]
fn build_tip(
    charge: i64,
    health: i64,
    lv: &Levels,
    status: &str,
    secs: i64,
    online: Option<u8>,
    cycles: Option<i64>,
    hdet_extra: &str,
    vendor: Option<&str>,
    tech: Option<&str>,
    voltage: Option<i64>,
    total_w: f64,
    pkg_w: Option<f64>,
    unc_w: Option<f64>,
    resid_w: Option<f64>,
    backlight: Option<i64>,
    history: Option<PowerStats>,
    weak: bool,
    manual: bool,
    pm_mode: powermode::Mode,
) -> String {
    let mut tip = String::new();
    tip.push_str(&kv(
        "Charge",
        &format!(
            "{} {}",
            bar(charge, grade(100 - charge, 70, 80), 20),
            mono(&format!("{charge:>3}%"))
        ),
    ));
    let mut cdet = format!(
        "{} / {} · {status}",
        uh(lv.now, lv.unit),
        uh(lv.full, lv.unit)
    );
    if secs > 0 {
        if status == "Charging" {
            cdet.push_str(&format!(" · {} until full", hdur(secs)));
        } else {
            cdet.push_str(&format!(" · {} left", hdur(secs)));
        }
    } else if online == Some(1) {
        cdet.push_str(" · on AC, not drawing");
    }
    tip.push_str(&kvsub(&cdet));

    tip.push_str(&kv(
        "Health",
        &format!(
            "{} {}",
            bar(health, grade(100 - health, 20, 35), 20),
            mono(&format!("{health:>3}%"))
        ),
    ));
    let mut hdet = format!("{}% worn", 100 - health);
    if let Some(c) = cycles.filter(|&c| c != 0) {
        hdet.push_str(&format!(" · {c} cycles"));
    }
    hdet.push_str(hdet_extra);
    tip.push_str(&kvsub(&hdet));

    if vendor.is_some() || tech.is_some() {
        tip.push_str(&kvsub(&esc(&format!(
            "{} {}",
            vendor.unwrap_or("?"),
            tech.unwrap_or("?")
        ))));
    }

    let mut pdet = if lv.rate > 0 {
        format!("{} draw", watts(lv.rate, voltage.unwrap_or(0), lv.unit))
    } else {
        "idle".to_string()
    };
    if let Some(v) = voltage {
        pdet.push_str(&format!(" · {:.2} V", v as f64 / 1_000_000.0));
        if lv.unit == "Ah" {
            pdet.push_str(&format!(" · {:.2} A", lv.rate as f64 / 1_000_000.0));
        }
    }
    tip.push_str(&kv("Power", &pdet));

    if let Some(h) = &history {
        let glyphs = crate::tooltip::heatbar(&h.buckets, 70, 90, 0);
        tip.push_str(&kvsub(&format!(
            "{glyphs}  {:.1}–{:.1} W over 1h, {:.1} W avg",
            h.min, h.max, h.avg
        )));
    }

    if let Some(pw) = pkg_w {
        let uw = unc_w.unwrap_or(0.0);
        let rw = resid_w.unwrap_or(0.0);
        tip.push_str(&kv(
            "Where",
            &format!("{} CPU {pw} W", bar(sharepct(pw, total_w), C_GOOD, 10)),
        ));
        tip.push_str(&kvsub(&format!(
            "{} GPU {uw} W",
            bar(sharepct(uw, total_w), C_GOOD, 10)
        )));
        let bl_suffix = backlight
            .map(|b| format!(" (backlight {b}%)"))
            .unwrap_or_default();
        tip.push_str(&kvsub(&format!(
            "{} rest {rw} W{bl_suffix}",
            bar(sharepct(rw, total_w), C_GOOD, 10)
        )));
    }

    if weak {
        tip.push_str(&kv("Mode", &format!("{} · weak charger", bad("eco"))));
    } else if manual {
        tip.push_str(&kv("Mode", &format!("{} · manual", pm_mode.as_str())));
    } else {
        let ac = if online == Some(1) {
            "on AC"
        } else {
            "on battery"
        };
        tip.push_str(&kv("Mode", &format!("{} · {ac}", pm_mode.as_str())));
    }

    // No in-body gesture line here — the HINTS footer ("click: toggle
    // power mode · right-click: powertop") already said this; a body line
    // repeating it was a duplicate, not a fallback.
    tip
}

// ---------------------------------------------------------- detached forks
//
// notify-send/paplay/powermode.sh must never block the event loop —
// powermode.sh does sudo, docker and brightness work and can take seconds,
// same reasoning ac-watch.sh backgrounds it with `&` for. tokio::spawn (not
// a bare process spawn) so the child is awaited and reaped rather than left
// a zombie.

fn spawn_detached(cmd: String, args: Vec<String>) {
    tokio::spawn(async move {
        let _ = Command::new(&cmd).args(&args).status().await;
    });
}

fn spawn_notify(summary: &str, body: &str) {
    spawn_detached(
        "notify-send".to_string(),
        vec![
            "-a".into(),
            "battery".into(),
            "-u".into(),
            "low".into(),
            "-h".into(),
            "string:x-canonical-private-synchronous:ac-watch".into(),
            summary.to_string(),
            body.to_string(),
        ],
    );
}

/// ac-watch.sh:43-55 — the muted check happens inside the spawned task, so
/// it never blocks the event loop either.
fn spawn_beep(sound: &'static str) {
    tokio::spawn(async move {
        if let Ok(out) = Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .await
        {
            if String::from_utf8_lossy(&out.stdout).contains("MUTED") {
                return;
            }
        }
        let f = format!("/usr/share/sounds/freedesktop/stereo/{sound}.oga");
        if Path::new(&f).is_file() {
            let _ = Command::new("paplay").arg(&f).status().await;
        }
    });
}

fn powermode_script_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.config/mango/scripts/powermode.sh")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------- levels/pct/watts

    #[test]
    fn levels_prefers_charge_type_and_falls_design_back_to_full() {
        let raw = RawBat {
            charge_now: Some(1_254_000),
            charge_full: Some(2_944_000),
            charge_full_design: Some(3_550_000),
            current_now: Some(1_067_000),
            ..Default::default()
        };
        let lv = levels(&raw).unwrap();
        assert_eq!(
            (lv.now, lv.full, lv.design, lv.rate, lv.unit),
            (1_254_000, 2_944_000, 3_550_000, 1_067_000, "Ah")
        );
    }

    #[test]
    fn levels_falls_back_to_energy_type_when_no_design_node() {
        let raw = RawBat {
            energy_now: Some(30_000_000),
            energy_full: Some(50_000_000),
            power_now: Some(8_000_000),
            ..Default::default()
        };
        let lv = levels(&raw).unwrap();
        assert_eq!(
            (lv.now, lv.full, lv.design, lv.rate, lv.unit),
            (30_000_000, 50_000_000, 50_000_000, 8_000_000, "Wh")
        );
    }

    #[test]
    fn levels_is_none_with_neither_charge_nor_energy() {
        assert!(levels(&RawBat::default()).is_none());
    }

    #[test]
    fn pct_rounds_half_up() {
        assert_eq!(pct(1_254_000, 2_944_000), 43);
        assert_eq!(pct(2_944_000, 3_550_000), 83);
        assert_eq!(pct(5, 0), 0);
    }

    #[test]
    fn uh_formats_two_decimals() {
        assert_eq!(uh(1_254_000, "Ah"), "1.25 Ah");
    }

    #[test]
    fn watts_matches_battery_sh_fixture() {
        assert_eq!(watts(1_067_000, 10_847_000, "Ah"), "11.6 W");
        assert_eq!(watts(8_000_000, 0, "Wh"), "8.0 W");
    }

    #[test]
    fn remaining_matches_battery_sh_fixture() {
        use crate::tooltip::hdur;
        assert_eq!(
            hdur(remaining(1_254_000, 2_944_000, 1_067_000, "Discharging")),
            "1h 10m"
        );
        assert_eq!(
            hdur(remaining(1_254_000, 2_944_000, 1_067_000, "Charging")),
            "1h 35m"
        );
        assert_eq!(remaining(1_254_000, 2_944_000, 0, "Discharging"), -1);
    }

    // ---------------------------------------------------------- icons/class

    #[test]
    fn ic_bat_walks_the_whole_table() {
        assert_eq!(ic_bat(0), '\u{f008e}');
        assert_eq!(ic_bat(100), '\u{f0079}');
        assert_eq!(ic_bat(50), '\u{f007e}');
        // T19: every bucket now has its own glyph — regression guard against
        // the old shared-glyph buckets (was 1/2 and 5/6, see ic_bat's doc
        // comment). One representative pct per bucket 0..=6.
        let all: Vec<char> = [0, 17, 34, 51, 68, 85, 100]
            .into_iter()
            .map(ic_bat)
            .collect();
        let unique: std::collections::HashSet<char> = all.iter().copied().collect();
        assert_eq!(unique.len(), 7, "expected 7 distinct glyphs: {all:?}");
    }

    #[test]
    fn class_for_critical_overrides_full_and_discharging_never_charging() {
        assert_eq!(class_for("Discharging", 15), "critical");
        assert_eq!(class_for("Full", 15), "critical");
        assert_eq!(class_for("Charging", 15), "charging");
        assert_eq!(class_for("Full", 100), "full");
        assert_eq!(class_for("Discharging", 50), "discharging");
    }

    #[test]
    fn icon_for_not_charging_keeps_plug_icon() {
        assert_eq!(icon_for("Not charging", 80), IC_CHG);
        assert_eq!(icon_for("Discharging", 80), ic_bat(80));
    }

    // -------------------------------------------------------------- RAPL

    #[test]
    fn rapl_watts_matches_battery_sh_fixture() {
        assert_eq!(
            rapl_watts(100_000, 0, 200_000, 1_000_000_000, 999_999_999),
            0.1
        );
        assert_eq!(
            rapl_watts(900_000, 0, 100_000, 1_000_000_000, 1_000_000),
            0.2
        );
        assert_eq!(rapl_watts(900_000, 0, 100_000, 1_000_000_000, 0), 0.0);
    }

    #[test]
    fn sharepct_matches_battery_sh_fixture() {
        assert_eq!(sharepct(5.0, 20.0), 25);
        assert_eq!(sharepct(30.0, 20.0), 100);
        assert_eq!(sharepct(5.0, 0.0), 0);
    }

    // -------------------------------------------------------------- wear

    #[test]
    fn wear_days_matches_battery_sh_fixture_table() {
        let nowd = 100 * 86400;
        assert_eq!(wear_days(84, 200, nowd - 90 * 86400, 100, nowd, 80), 45);
        assert_eq!(wear_days(80, 200, nowd - 90 * 86400, 100, nowd, 80), -2);
        assert_eq!(wear_days(75, 200, nowd - 90 * 86400, 100, nowd, 80), -2);
        assert_eq!(wear_days(84, 200, nowd - 3 * 86400, 190, nowd, 80), -1);
        assert_eq!(wear_days(84, 200, nowd - 90 * 86400, 200, nowd, 80), -1);
        assert_eq!(wear_days(100, 200, nowd - 90 * 86400, 100, nowd, 80), -3);
    }

    #[test]
    fn ddur_matches_battery_sh_fixture() {
        assert_eq!(ddur(20), "~20d");
        assert_eq!(ddur(45), "~1mo");
        assert_eq!(ddur(480), "~1y 3mo");
        assert_eq!(ddur(365), "~1y");
    }

    #[test]
    fn wear_prune_keeps_only_rows_inside_the_window() {
        let nowd = 100 * 86400;
        let old = WearRow {
            epoch: nowd - 91 * 86400,
            cycles: 100,
            full: 2_900_000,
        };
        let inside = WearRow {
            epoch: nowd - 10 * 86400,
            cycles: 150,
            full: 2_900_000,
        };
        assert!(!wear_survives_prune(&old, nowd, WEAR_WINDOW));
        assert!(wear_survives_prune(&inside, nowd, WEAR_WINDOW));
    }

    #[test]
    fn matches_hist_file_rejects_decoys() {
        assert!(matches_hist_file(
            "history-rate-X421-35-42-123456789.dat",
            "X421-35"
        ));
        assert!(!matches_hist_file("history-rate-generic_id.dat", "X421-35"));
        assert!(!matches_hist_file(
            "history-rate-ThinkPad_Keyboard-aa:bb.dat",
            "X421-35"
        ));
    }

    // ------------------------------------------------------- power_stats

    #[test]
    fn power_stats_matches_battery_sh_fixture() {
        let nowt = 1_700_000_000;
        let samples = vec![
            PowerSample {
                epoch: nowt - 3500,
                watts: 10.0,
                state: "discharging".into(),
            },
            PowerSample {
                epoch: nowt - 3000,
                watts: 15.0,
                state: "discharging".into(),
            },
            PowerSample {
                epoch: nowt - 1800,
                watts: 5.0,
                state: "charging".into(),
            },
            PowerSample {
                epoch: nowt - 1200,
                watts: 20.0,
                state: "discharging".into(),
            },
            PowerSample {
                epoch: nowt - 100,
                watts: 50.0,
                state: "unknown".into(),
            },
            PowerSample {
                epoch: nowt - 7200,
                watts: 999.0,
                state: "discharging".into(),
            },
        ];
        let stats = power_stats(&samples, nowt, 3600, 24).unwrap();
        assert_eq!((stats.min, stats.max, stats.avg), (10.0, 20.0, 15.0));
        assert_eq!(stats.buckets.len(), 24);
    }

    #[test]
    fn power_stats_is_none_when_nothing_matches() {
        assert!(power_stats(&[], 1_700_000_000, 3600, 24).is_none());
    }

    // -------------------------------------------------------------- ac-watch

    #[test]
    fn ac_changed_matches_ac_watch_sh_selftest_table() {
        assert!(ac_changed(Some(0), Some(1)));
        assert!(ac_changed(Some(1), Some(0)));
        assert!(!ac_changed(Some(1), Some(1)));
        assert!(!ac_changed(Some(0), Some(0)));
        assert!(!ac_changed(Some(0), None));
    }

    #[test]
    fn ac_wording_matches_ac_watch_sh_fixture() {
        assert_eq!(ac_summary(1), "Charger connected");
        assert_eq!(ac_summary(0), "On battery");
        assert_eq!(ac_body(1, Some(73)), "73% · charging");
        assert_eq!(ac_body(0, Some(73)), "73% · discharging");
        assert_eq!(ac_body(1, None), "");
    }
}
