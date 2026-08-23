//! Claude usage pill: renders the OAuth usage data the `claudebar` binary
//! already fetches, instead of re-implementing its OAuth refresh and
//! credential write-back. T6c (see IRONBAR.md).
//!
//! **Correction to this crate's own prior claim:** `IRONBAR.md`/
//! `tooltip.sh`'s comments call claudebar "a Rust binary". It is not —
//! `/usr/bin/claudebar` is a 1179-line bash script (AUR package `claudebar`)
//! that reads `~/.claude/.credentials.json`, calls the usage API, refreshes
//! and rewrites the OAuth token under a file lock, and caches the raw API
//! response at `~/.cache/claudebar/usage.json`. There is no Rust precedent
//! to reuse; there is also no "plugin" — it is a third-party system package.
//!
//! Split at that cache file: `claudebar` keeps doing the delicate half
//! (OAuth refresh, credential write-back, retry budgets, staleness
//! markers) on its own cadence; `mango-bard` only reads the cache file it
//! already writes and renders both the pill and the popup with
//! `tooltip.rs`, so the popup is matugen-themed like every other popup on
//! this bar instead of claudebar's own hardcoded One Dark palette.
//! **This module never reads or writes `~/.claude/.credentials.json`.**
//!
//! Numbers are ported verbatim from `/usr/bin/claudebar`: session/weekly
//! utilization (`parse_pct`, claudebar:619-621), `limits[]`'s
//! `weekly_scoped` rows capped at 4 (claudebar:679-690), the extra-usage
//! cents->dollars conversion (claudebar:737-748), the point-based pacing
//! arrow (claudebar:342-384's ratio-icon half — the default, non-
//! `--tooltip-pace-pts` tooltip only ever shows this arrow, never the
//! "Npts ahead" text, which is a `--format`/`--tooltip-format` placeholder
//! this popup has no equivalent of), and the low/mid/high/critical class
//! thresholds (claudebar:826-839).
//!
//! Not ported: the `plan` label ("Claude Max 5x") — that comes from
//! `~/.claude/.credentials.json`'s `subscriptionType`/`rateLimitTier`,
//! which this module deliberately never reads; the popup title is the
//! generic "Claude Usage" instead. Also not ported: `seven_day_sonnet`'s
//! dedicated field/dedup path — the real payload captured this session had
//! it `null`, and the modern per-model path (`limits[]`'s `weekly_scoped`
//! rows, e.g. "Fable only") already covers a scoped weekly window
//! generically, so porting a second, legacy path for the same concept
//! would be dead code on this account.

use crate::mango::CLASS_PREFIX;
use crate::tooltip::{bar, barico_label, dim, esc, grade, row, rule, sect, set_titled};
use crate::vars::Vars;
use serde_json::Value;
use std::path::PathBuf;

/// T16: nf-md-robot, Plane-15 PUA — see `docker.rs`'s `IC_DOCKER` doc
/// comment for why this range is safe from the T8b fontconfig collision.
/// `barico_label()` needs `.claudebar` in style.css's Nerd Font selector
/// list to actually render it; without that this falls through to IBM Plex
/// Sans and draws tofu.
const IC_CLAUDE: char = '\u{f06a9}';

/// claudebar:83 `PACE_TOLERANCE=5`.
const PACE_TOLERANCE: i64 = 5;
/// claudebar:223-224.
const SESSION_WINDOW: i64 = 5 * 3600;
const WEEKLY_WINDOW: i64 = 7 * 24 * 3600;
/// claudebar:220 `BAR_LEN=20`.
const METER_CELLS: usize = 20;

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

fn cache_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join(".cache/claudebar")
}

fn now_epoch() -> i64 {
    // SAFETY: libc::time with a null out-param only reads the kernel clock —
    // no aliasing or lifetime hazard.
    unsafe { libc::time(std::ptr::null_mut()) }
}

/// True on every 5th minute. T6c's fetch rides the existing clock tick
/// (main.rs) instead of arming a new timerfd, matching IRONBAR.md's "300s
/// aligned, all modes" period at zero extra timer cost — only the poke this
/// gates adds a wakeup, once per 5 real minutes.
pub fn due_on_tick() -> bool {
    (now_epoch() / 60) % 5 == 0
}

/// Spawns `claudebar` in the background and pokes the control socket to
/// render once it exits. Must never be awaited inline: claudebar's own
/// retry budget runs up to ~20s on a cold cache (network wait), unlike
/// docker.rs's bounded `docker ps` fork — blocking the daemon's event loop
/// for that long would freeze every other pill's IPC flush too. Its stdout
/// (a `{text,tooltip,class}` JSON claudebar renders for itself) is
/// discarded — only the cache file it writes as a side effect matters here.
pub fn spawn_fetch() {
    tokio::spawn(async {
        let _ = tokio::process::Command::new("claudebar").output().await;
        let _ = crate::control::send("refresh claude").await;
    });
}

/// Parses `"2026-08-22T14:00:00.399038+00:00"` (the one shape the usage API
/// returns) into a Unix epoch. No chrono: this is the only *foreign*
/// timestamp the crate parses (clock.rs's own calendar math is always
/// local-now, never a value read from elsewhere), so a hand-rolled parser
/// matching the one observed shape is cheaper than a new dependency for it.
fn parse_rfc3339(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let year: i32 = s.get(0..4)?.parse().ok()?;
    let month: i32 = s.get(5..7)?.parse().ok()?;
    let day: i32 = s.get(8..10)?.parse().ok()?;
    let hour: i32 = s.get(11..13)?.parse().ok()?;
    let min: i32 = s.get(14..16)?.parse().ok()?;
    let sec: i32 = s.get(17..19)?.parse().ok()?;
    let rest = &s[19..];
    let offset_secs: i64 = if rest.contains('Z') {
        0
    } else if let Some(idx) = rest.find(['+', '-']) {
        let sign: i64 = if rest.as_bytes()[idx] == b'-' { -1 } else { 1 };
        let off = &rest[idx + 1..];
        let oh: i64 = off.get(0..2)?.parse().ok()?;
        let om: i64 = off.get(3..5).unwrap_or("00").parse().unwrap_or(0);
        sign * (oh * 3600 + om * 60)
    } else {
        0
    };
    // SAFETY: `tm` fields are plain integers from the parse above;
    // `libc::timegm` reads them and returns a UTC epoch with no DST/timezone
    // lookup — no aliasing hazard.
    let epoch = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = year - 1900;
        tm.tm_mon = month - 1;
        tm.tm_mday = day;
        tm.tm_hour = hour;
        tm.tm_min = min;
        tm.tm_sec = sec;
        libc::timegm(&mut tm)
    };
    Some(epoch - offset_secs)
}

/// claudebar:619-621 `parse_pct()`: utilization, rounded, clamped to
/// `0..=1e12` (a hostile/garbage value resets to 0 rather than propagating).
fn pct_of(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|w| w.get("utilization"))
        .and_then(Value::as_f64)
        .map(|f| f.round() as i64)
        .filter(|&n| (0..=1_000_000_000_000).contains(&n))
        .unwrap_or(0)
}

fn reset_of(v: &Value, key: &str) -> Option<i64> {
    v.get(key)
        .and_then(|w| w.get("resets_at"))
        .and_then(Value::as_str)
        .and_then(parse_rfc3339)
}

struct Scoped {
    name: String,
    pct: i64,
    resets_at: Option<i64>,
}

/// claudebar:679-690 — `weekly_scoped` rows from `limits[]`, capped at 4 so a
/// hostile/huge payload can't grow the popup or the parse cost unbounded.
fn scoped_limits(usage: &Value) -> Vec<Scoped> {
    let Some(limits) = usage.get("limits").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for l in limits {
        if out.len() >= 4 {
            break;
        }
        if l.get("kind").and_then(Value::as_str) != Some("weekly_scoped") {
            continue;
        }
        let Some(model) = l.get("scope").and_then(|s| s.get("model")) else {
            continue;
        };
        let name = model
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or("Model")
            .to_string();
        let pct = l
            .get("percent")
            .and_then(Value::as_f64)
            .map(|f| f.round() as i64)
            .unwrap_or(0)
            .max(0);
        let resets_at = l
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339);
        out.push(Scoped {
            name,
            pct,
            resets_at,
        });
    }
    out
}

struct Extra {
    show: bool,
    spent_cents: i64,
    limit_cents: i64,
    pct: i64,
}

/// claudebar:717-748 — integer cents throughout. Shown whenever real spend
/// data exists, not gated on `is_enabled`: the API reports that false once
/// the balance is depleted, but `spend` is still real and worth showing
/// (claudebar's own comment at :718-721 makes the same call).
fn extra_usage(usage: &Value) -> Extra {
    let Some(eu) = usage.get("extra_usage") else {
        return Extra {
            show: false,
            spent_cents: 0,
            limit_cents: 0,
            pct: 0,
        };
    };
    let limit = eu
        .get("monthly_limit")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .floor() as i64;
    let used = eu
        .get("used_credits")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .floor() as i64;
    let show = limit > 0 || used > 0;
    let pct = if limit > 0 { used * 100 / limit } else { 0 };
    Extra {
        show,
        spent_cents: used.max(0),
        limit_cents: limit.max(0),
        pct,
    }
}

fn dollars(cents: i64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

/// claudebar:826-839 — rate limits first; extra usage only counts once a
/// rate limit itself has hit 100%.
fn class_for(session_pct: i64, weekly_pct: i64, scoped_max: i64, extra_pct: i64) -> &'static str {
    let mut max_pct = session_pct.max(weekly_pct).max(scoped_max);
    if session_pct >= 100 || weekly_pct >= 100 || scoped_max >= 100 {
        max_pct = max_pct.max(extra_pct);
    }
    if max_pct >= 90 {
        "critical"
    } else if max_pct >= 75 {
        "high"
    } else if max_pct >= 50 {
        "mid"
    } else {
        "low"
    }
}

/// claudebar:342-384's ratio-based pacing icon (the one the default, non-
/// `--tooltip-pace-pts` tooltip actually renders).
fn pace_icon(pct: i64, resets_at: Option<i64>, window_s: i64, now: i64) -> char {
    let Some(reset) = resets_at else {
        return '\u{2192}';
    };
    if window_s <= 0 {
        return '\u{2192}';
    }
    let remaining = reset - now;
    let elapsed_pct = ((window_s - remaining) * 100 / window_s).clamp(0, 100);
    if elapsed_pct <= 0 {
        return '\u{2192}';
    }
    let pacing_x100 = pct * 100 / elapsed_pct;
    if pacing_x100 > 100 + PACE_TOLERANCE {
        '\u{2191}'
    } else if pacing_x100 < 100 - PACE_TOLERANCE {
        '\u{2193}'
    } else {
        '\u{2192}'
    }
}

/// claudebar:315-331 `countdown()` — day-granular past 24h, hour:minute
/// under it, "now" once past the deadline, an em dash with no reset time.
fn countdown(resets_at: Option<i64>, now: i64) -> String {
    let Some(reset) = resets_at else {
        return "\u{2014}".to_string();
    };
    let diff = reset - now;
    if diff <= 0 {
        return "now".to_string();
    }
    let d = diff / 86400;
    let h = diff % 86400 / 3600;
    let m = diff % 3600 / 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else {
        format!("{h}h {m:02}m")
    }
}

/// Formats a `SystemTime` as local `HH:MM` — the cache file's mtime, for the
/// popup's "Updated" footer. A private, single-use twin of clock.rs's own
/// `format_local`: that helper always reads the *current* wall clock, with
/// no parameter for an arbitrary instant, so generalizing its signature for
/// this one caller would touch a file T1 through T8c never had to change,
/// for a net cost bigger than repeating its ~6-line libc idiom here.
fn format_hhmm(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // SAFETY: `secs` is a plain integer; `localtime_r` fills a zeroed
    // out-param in place — no aliasing hazard.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

/// One meter section: label, bar, pct + pace arrow, then a dim "Resets in
/// ..." line. `grade(pct, 75, 90)` collapses claudebar's own 4-tier meter
/// color (green/yellow/orange/red) to tooltip.rs's existing 3-tier
/// good/warn/bad — ponytail: a true 4th tier would need its own color
/// constant for this one caller; upgrade path is a local ORANGE const plus a
/// 4-way match, if the collapsed 75/90 boundary ever needs to read
/// distinctly from the pill's own low/mid/high/critical class.
fn meter_section(label: &str, pct: i64, resets_at: Option<i64>, window_s: i64, now: i64) -> String {
    let colour = grade(pct, 75, 90);
    let icon = pace_icon(pct, resets_at, window_s, now);
    let mut out = sect("", label);
    out.push_str(&row(&format!(
        "{}  <span font_weight=\"bold\">{pct}% {icon}</span>",
        bar(pct, colour, METER_CELLS)
    )));
    out.push('\n');
    out.push_str(&dim(&format!("Resets in {}", countdown(resets_at, now))));
    out.push('\n');
    out
}

pub struct Claude;

impl Claude {
    pub fn new() -> Self {
        Self
    }

    /// Pure file read + render, no fork — unlike [`spawn_fetch`]. Safe to
    /// call inline from the popup's own click (T7a's pattern) or the
    /// control socket.
    pub fn refresh(&mut self, vars: &mut Vars) {
        let path = cache_dir().join("usage.json");
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => {
                vars.set("claude_text", "\u{26a0}");
                set_titled(
                    vars,
                    "claude_tip",
                    "Claude Usage",
                    "No usage data yet.\nRun <b>claude</b> to log in.".to_string(),
                );
                vars.set(&class_key("claudebar"), "critical");
                return;
            }
        };
        let usage: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => {
                vars.set("claude_text", "\u{26a0}");
                set_titled(
                    vars,
                    "claude_tip",
                    "Claude Usage",
                    "Usage cache is not valid JSON.".to_string(),
                );
                vars.set(&class_key("claudebar"), "critical");
                return;
            }
        };

        let now = now_epoch();
        let session_pct = pct_of(&usage, "five_hour");
        let session_reset = reset_of(&usage, "five_hour");
        let weekly_pct = pct_of(&usage, "seven_day");
        let weekly_reset = reset_of(&usage, "seven_day");
        let scoped = scoped_limits(&usage);
        let scoped_max = scoped.iter().map(|s| s.pct).max().unwrap_or(0);
        let extra = extra_usage(&usage);
        let class = class_for(session_pct, weekly_pct, scoped_max, extra.pct);
        let stale = cache_dir().join(".stale").exists();

        vars.set(
            "claude_text",
            format!(
                "{} {session_pct}% \u{b7} {}{}",
                barico_label(IC_CLAUDE),
                countdown(session_reset, now),
                if stale { " \u{23f8}" } else { "" }
            ),
        );
        vars.set(&class_key("claudebar"), class);

        let mut tip = String::new();
        tip.push_str(&meter_section(
            "Session",
            session_pct,
            session_reset,
            SESSION_WINDOW,
            now,
        ));
        tip.push_str(&meter_section(
            "Weekly",
            weekly_pct,
            weekly_reset,
            WEEKLY_WINDOW,
            now,
        ));
        for s in &scoped {
            tip.push_str(&meter_section(
                &format!("{} only", esc(&s.name)),
                s.pct,
                s.resets_at,
                WEEKLY_WINDOW,
                now,
            ));
        }
        if extra.show {
            tip.push_str(&rule(34));
            tip.push_str(&sect("", "Extra usage"));
            let colour = grade(extra.pct, 75, 90);
            tip.push_str(&row(&format!(
                "{}  <span font_weight=\"bold\">{}</span>",
                bar(extra.pct, colour, METER_CELLS),
                dollars(extra.spent_cents)
            )));
            tip.push('\n');
            tip.push_str(&dim(&format!("Limit: {}", dollars(extra.limit_cents))));
            tip.push('\n');
        }
        tip.push_str(&rule(34));

        let updated = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .map(format_hhmm)
            .unwrap_or_else(|| "\u{2014}".to_string());
        if stale {
            tip.push_str(&dim(&format!(
                "\u{23f8}  Stale \u{2014} data from {updated}"
            )));
        } else {
            tip.push_str(&dim(&format!("Updated {updated}")));
        }

        set_titled(
            vars,
            "claude_tip",
            "Claude Usage",
            tip.trim_end_matches('\n').to_string(),
        );
    }
}

impl Default for Claude {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rfc3339_reads_utc_offset() {
        assert_eq!(
            parse_rfc3339("2026-08-22T14:00:00.399038+00:00"),
            Some(parse_rfc3339("2026-08-22T14:00:00+00:00").unwrap())
        );
    }

    #[test]
    fn parse_rfc3339_honors_nonzero_offset() {
        // Same wall-clock instant, one hour behind UTC: the +01:00 form must
        // resolve to one hour earlier in epoch terms.
        let utc = parse_rfc3339("2026-08-22T14:00:00Z").unwrap();
        let plus_one = parse_rfc3339("2026-08-22T15:00:00+01:00").unwrap();
        assert_eq!(utc, plus_one);
    }

    #[test]
    fn parse_rfc3339_rejects_short_garbage() {
        assert_eq!(parse_rfc3339("not-a-date"), None);
    }

    #[test]
    fn pct_of_reads_utilization_and_clamps_out_of_range() {
        let v: Value = serde_json::from_str(r#"{"five_hour":{"utilization":51.0}}"#).unwrap();
        assert_eq!(pct_of(&v, "five_hour"), 51);
        let bad: Value = serde_json::from_str(r#"{"five_hour":{"utilization":-5}}"#).unwrap();
        assert_eq!(
            pct_of(&bad, "five_hour"),
            0,
            "claudebar:620 resets < 0 to 0"
        );
        let missing: Value = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(pct_of(&missing, "five_hour"), 0);
    }

    #[test]
    fn scoped_limits_caps_at_four_and_skips_non_scoped_kinds() {
        let usage: Value = serde_json::from_str(
            r#"{"limits":[
                {"kind":"session","percent":10},
                {"kind":"weekly_scoped","percent":14,"resets_at":"2026-08-24T00:00:00Z",
                 "scope":{"model":{"display_name":"Fable"}}},
                {"kind":"weekly_scoped","percent":20,"scope":{"model":{"display_name":"A"}}},
                {"kind":"weekly_scoped","percent":21,"scope":{"model":{"display_name":"B"}}},
                {"kind":"weekly_scoped","percent":22,"scope":{"model":{"display_name":"C"}}},
                {"kind":"weekly_scoped","percent":23,"scope":{"model":{"display_name":"D"}}}
            ]}"#,
        )
        .unwrap();
        let scoped = scoped_limits(&usage);
        assert_eq!(scoped.len(), 4, "capped at 4 (claudebar:679-690)");
        assert_eq!(scoped[0].name, "Fable");
        assert_eq!(scoped[0].pct, 14);
        assert!(scoped[0].resets_at.is_some());
    }

    #[test]
    fn scoped_limits_skips_rows_with_no_model_scope() {
        let usage: Value =
            serde_json::from_str(r#"{"limits":[{"kind":"weekly_scoped","percent":50}]}"#).unwrap();
        assert!(scoped_limits(&usage).is_empty());
    }

    #[test]
    fn extra_usage_converts_cents_and_computes_pct() {
        let v: Value =
            serde_json::from_str(r#"{"extra_usage":{"monthly_limit":2500,"used_credits":2471.0}}"#)
                .unwrap();
        let e = extra_usage(&v);
        assert!(e.show);
        assert_eq!(e.spent_cents, 2471);
        assert_eq!(e.limit_cents, 2500);
        assert_eq!(e.pct, 98, "2471*100/2500 = 98 (claudebar:746)");
        assert_eq!(dollars(e.spent_cents), "$24.71");
        assert_eq!(dollars(e.limit_cents), "$25.00");
    }

    #[test]
    fn extra_usage_hidden_when_no_real_spend_data() {
        let v: Value = serde_json::from_str(r#"{}"#).unwrap();
        assert!(!extra_usage(&v).show);
        let zero: Value =
            serde_json::from_str(r#"{"extra_usage":{"monthly_limit":0,"used_credits":0}}"#)
                .unwrap();
        assert!(!extra_usage(&zero).show);
    }

    #[test]
    fn class_for_matches_claudebar_thresholds() {
        assert_eq!(class_for(10, 20, 0, 0), "low");
        assert_eq!(class_for(51, 20, 0, 0), "mid");
        assert_eq!(class_for(80, 20, 0, 0), "high");
        assert_eq!(class_for(95, 20, 0, 0), "critical");
    }

    #[test]
    fn class_for_folds_in_extra_only_once_a_limit_hits_100() {
        // claudebar:830-832: extra only counts once a rate limit itself is
        // already at 100 — a high extra pct alone must not raise the class.
        assert_eq!(class_for(50, 50, 0, 99), "mid");
        assert_eq!(class_for(100, 50, 0, 99), "critical");
    }

    #[test]
    fn pace_icon_flags_ahead_behind_and_on_track() {
        let now = 1_000_000;
        let window = 1000;
        // 60% used, 50% elapsed -> pacing_x100 = 120 > 105 -> ahead.
        assert_eq!(
            pace_icon(60, Some(now + window / 2), window, now),
            '\u{2191}'
        );
        // 20% used, 50% elapsed -> pacing_x100 = 40 < 95 -> under.
        assert_eq!(
            pace_icon(20, Some(now + window / 2), window, now),
            '\u{2193}'
        );
        // No reset time at all -> neutral.
        assert_eq!(pace_icon(50, None, window, now), '\u{2192}');
    }

    #[test]
    fn countdown_formats_days_then_hours_then_now() {
        let now = 1_000_000;
        assert_eq!(countdown(Some(now + 2 * 86400 + 3600), now), "2d 1h");
        assert_eq!(countdown(Some(now + 3600 + 120), now), "1h 02m");
        assert_eq!(countdown(Some(now - 1), now), "now");
        assert_eq!(countdown(None, now), "\u{2014}");
    }

    #[test]
    fn due_on_tick_is_a_pure_function_of_the_5_minute_boundary() {
        // Can't control the real clock in a unit test, so this only checks
        // the function is callable and returns a bool — the cadence itself
        // is verified live (mango-bard stats' claude_fetches counter, see
        // IRONBAR.md's T6c verification section).
        let _ = due_on_tick();
    }
}
