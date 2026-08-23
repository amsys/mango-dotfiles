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

/// T8c: markup restored to match clock.sh's own `BIG`/date span
/// (clock.sh:480 `BIG="<span size='115%' font_weight='600'>$TIME</span>"`) —
/// the port had dropped it, which is why the clock ran together with no
/// visible weight against its neighbours.
///
/// T15: date's own `• ` prefix (clock.sh:444, "the date lives in custom/date
/// now... the '• ' separator moved over with it") is dropped — clock and
/// date have been two separate pills with their own spacing since T8c split
/// them; T14 also tightened the row's own gaps, so the leftover dot from a
/// one-pill-shared-text era now reads as a stray mark, not a separator.
pub fn refresh(vars: &mut Vars) {
    vars.set(
        "clk_text",
        format!(
            "<span size=\"115%\" font_weight=\"600\">{}</span>",
            format_local("%H:%M")
        ),
    );
    vars.set(
        "date_text",
        format!("<span alpha=\"70%\">{}</span>", format_local("%a, %d %b")),
    );
}

// ---------------------------------------------------------- T-next: popups
//
// Ports clock.sh's two tooltip bodies (clock.sh:230-274 world clocks,
// clock.sh:195-208 calendar) as click popups — ironbar 0.19.0 has no
// dynamic hover tooltip (IRONBAR.md), so every rich detail this bar shows
// is a click popup, not a hover. Both builders here are only ever called
// from the control socket's `clock-detail`/`date-detail` topics, which are
// only ever poked from inside the popup's own `on_click_left` script while
// it is open (genconfig.rs, same D1/D2 shape T7a's cpu/memory popups
// already established) — nothing in this section runs at idle.
//
// No `$XDG_RUNTIME_DIR` tip cache, unlike clock.sh's `TIP_CACHE`: that
// cache exists only because clock.sh re-execs on every poll; a long-lived
// daemon that only builds a tip on a real click has nothing to amortise
// (T5/T6b/T7a already made this same call for their own popups).

/// clock.sh:61-67 `tz_label()` — "America/New_York" -> "New York". A few
/// zones are named after the country rather than the city they mean; those
/// get a literal override.
fn tz_label(zone: &str) -> String {
    if zone == "Indian/Mauritius" {
        return "Port Louis".to_string();
    }
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

/// clock.sh:70 `local_zone()` — the zone `/etc/localtime` points at. A
/// plain symlink read (`realpath` in the shell is one `readlink` syscall
/// under the hood), so this forks nothing. Empty string on any read
/// failure, matching the shell's `2> /dev/null` swallow.
fn local_zone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .and_then(|s| s.split("/zoneinfo/").nth(1).map(str::to_string))
        .unwrap_or_default()
}

/// clock.sh:73-82 `tz_offset()` — "+0400" vs "+0100" -> "-3h" / "+2h30" /
/// "same". Ports the fixed-column `%z` parse (`${r:1:2}`/`${r:3:2}`) as a
/// byte slice — Rust's `str::parse` has no bash-style octal trap on a
/// leading zero, so unlike the shell this needs no `10#` base prefix.
fn tz_offset(remote: &str, local: &str) -> String {
    fn minutes(z: &str) -> i32 {
        let sign = if z.starts_with('-') { -1 } else { 1 };
        let h: i32 = z.get(1..3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let m: i32 = z.get(3..5).and_then(|s| s.parse().ok()).unwrap_or(0);
        sign * (h * 60 + m)
    }
    let d = minutes(remote) - minutes(local);
    if d == 0 {
        return "same".to_string();
    }
    let sign = if d < 0 { "-" } else { "+" };
    let d = d.abs();
    if d % 60 == 0 {
        format!("{sign}{}h", d / 60)
    } else {
        format!("{sign}{}h{:02}", d / 60, d % 60)
    }
}

/// clock.sh:50-55 `ctr()` — centres `text` in a `width`-character cell of
/// plain spaces, the smaller half of an odd pad going on the left (matches
/// awk's `int(p/2)` / `p-int(p/2)` split). Padding stays outside any markup
/// so it never counts a span tag as display width.
fn ctr(text: &str, width: usize) -> String {
    let len = text.chars().count();
    let pad = width.saturating_sub(len);
    let left = pad / 2;
    let right = pad - left;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(right))
}

const WORLD_TZ_DEFAULT: &str = "America/New_York,Europe/Prague,Asia/Bangkok,Indian/Mauritius";

fn world_zones() -> String {
    std::env::var("MANGO_WORLD_TZ").unwrap_or_else(|_| WORLD_TZ_DEFAULT.to_string())
}

/// Forks `date` with a per-process `TZ` (`Command::env` — scoped to this one
/// child, never the daemon's own environment) rather than mutating the
/// daemon's process-wide `TZ`, which would race the daemon's own
/// `localtime_r` calls (clock.sh's `TZ="$z" printf ...` subshell has no such
/// hazard; a long-lived multi-threaded-capable daemon does). One fork per
/// remote zone, combining `%z`/`%j`/`%H:%M` into a single invocation rather
/// than three, since all three are read from the same child either way.
async fn zone_snapshot(zone: &str) -> Option<(String, i32, String)> {
    let out = tokio::process::Command::new("date")
        .env("TZ", zone)
        .arg("+%z %j %H:%M")
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut parts = text.split_whitespace();
    let z = parts.next()?.to_string();
    let day: i32 = parts.next()?.parse().ok()?;
    let time = parts.next()?.to_string();
    Some((z, day, time))
}

/// clock.sh:249-251's day-wrap suffix: `%j` (day of year) wraps at New
/// Year's, so the "yesterday/tomorrow" call compares the raw difference
/// both ways rather than just its sign.
fn day_wrap_suffix(remote_day: i32, local_day: i32) -> &'static str {
    if remote_day - local_day == 1 || local_day - remote_day > 300 {
        " +1d"
    } else {
        " -1d"
    }
}

/// clock.sh:230-274 — title (full date), `Local` section (250%-scale time,
/// zone label + `%z`), `World` section (3 zones per row, time/label/offset
/// stacked). `clk_tip` ironvar for the click popup genconfig.rs wires up.
pub async fn refresh_clock_tip(vars: &mut Vars) {
    use crate::tooltip::{dim, mono, row, sect, set_titled, C_DIM, C_LABEL};

    const IC_LOCAL: char = '\u{f0954}'; // clock.sh:43, Nerd Font MDI md-clock
    const IC_EARTH: char = '\u{f01e7}'; // clock.sh:44, Nerd Font MDI md-earth
    const COL_W: usize = 15;
    const COL_GAP: usize = 3;
    const T_W: usize = 10;
    const T_GAP: usize = 2;
    const TIME_SCALE: u32 = 150;
    const WORLD_COLS: usize = 3;

    let local_time = format_local("%H:%M");
    let local_z = format_local("%z");
    let local_day: i32 = format_local("%j").trim().parse().unwrap_or(0);
    let local_zone_name = local_zone();

    let title_text = format_local("%A, %d %B %Y");
    let mut tip = String::new();
    tip.push_str(&sect(&IC_LOCAL.to_string(), "Local"));
    tip.push_str(&row(&mono(&format!(
        "<span size='250%' font_weight='bold'>{local_time}</span>"
    ))));
    tip.push('\n');
    tip.push_str(&dim(&format!(
        "{} \u{b7} {local_z}",
        tz_label(&local_zone_name)
    )));
    tip.push('\n');
    tip.push_str(&sect(&IC_EARTH.to_string(), "World"));

    // Zones first (up to 4 forks — fewer whenever the local zone is one of
    // them, same as clock.sh's own `[ "$z" != "$LOCALZONE" ]` skip), then
    // laid out a rowful at a time.
    let world = world_zones();
    let mut times = Vec::new();
    let mut labels = Vec::new();
    let mut notes = Vec::new();
    for zone in world
        .split(',')
        .filter(|z| !z.is_empty() && *z != local_zone_name)
    {
        let Some((zz, zd, zt)) = zone_snapshot(zone).await else {
            continue;
        };
        let mut off = tz_offset(&zz, &local_z);
        if off == "same" {
            off = "same time".to_string();
        }
        if zd != local_day {
            off.push_str(day_wrap_suffix(zd, local_day));
        }
        times.push(zt);
        labels.push(tz_label(zone));
        notes.push(off);
    }

    for chunk_start in (0..times.len()).step_by(WORLD_COLS) {
        let end = (chunk_start + WORLD_COLS).min(times.len());
        let t_row = times[chunk_start..end]
            .iter()
            .map(|t| ctr(t, T_W))
            .collect::<Vec<_>>()
            .join(&" ".repeat(T_GAP));
        let l_row = labels[chunk_start..end]
            .iter()
            .map(|l| ctr(l, COL_W))
            .collect::<Vec<_>>()
            .join(&" ".repeat(COL_GAP));
        let n_row = notes[chunk_start..end]
            .iter()
            .map(|n| ctr(n, COL_W))
            .collect::<Vec<_>>()
            .join(&" ".repeat(COL_GAP));
        tip.push_str(&row(&mono(&format!(
            "<span size='{TIME_SCALE}%' font_weight='bold'>{t_row}</span>"
        ))));
        tip.push('\n');
        tip.push_str(&row(&mono(&format!(
            "<span foreground=\"{C_LABEL}\">{l_row}</span>"
        ))));
        tip.push('\n');
        tip.push_str(&row(&mono(&format!(
            "<span foreground=\"{C_DIM}\">{n_row}</span>"
        ))));
        tip.push('\n');
    }

    set_titled(vars, "clk_tip", &title_text, tip);
}

/// clock.sh:96-127 `cal_grid()` — forks `cal -m [month year]` and reads its
/// fixed 3-character columns exactly as the shell's awk does (matching the
/// existing shell rather than re-deriving the calendar keeps its verbatim
/// self-check fixture usable, and is one fork on click only). `today = 0`
/// never matches a real day, so passing it highlights nothing — not used by
/// `refresh_date_tip` but kept for a future "jump to month" popup control.
async fn cal_grid(today: u32, month_year: Option<(u32, i32)>) -> String {
    let mut args = vec!["-m".to_string()];
    if let Some((m, y)) = month_year {
        args.push(m.to_string());
        args.push(y.to_string());
    }
    let out = tokio::process::Command::new("cal")
        .args(&args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    cal_grid_render(&out, today)
}

/// Pure render half of [`cal_grid`], split out so the self-check can feed it
/// a captured `cal -m` fixture with no fork — clock.sh:97-126's awk body.
fn cal_grid_render(cal_out: &str, today: u32) -> String {
    use crate::tooltip::{C_DIM, C_LABEL};

    let mut lines = cal_out.lines();
    let _month_name = lines.next(); // NR==1, skipped — the popup's own title widget already shows this.
    let Some(header) = lines.next() else {
        return String::new();
    };

    let mut out = vec![
        border_row("\u{256d}", "\u{252c}", "\u{256e}"), // ╭┬╮
        cell_row(header, C_DIM, false, today),
        border_row("\u{251c}", "\u{253c}", "\u{2524}"), // ├┼┤
    ];
    for line in lines {
        // Guard: `cal` can emit a trailing blank line.
        if line.bytes().any(|b| b.is_ascii_digit()) {
            out.push(cell_row(line, C_LABEL, true, today));
        }
    }
    out.push(border_row("\u{2570}", "\u{2534}", "\u{256f}")); // ╰┴╯
    out.join("\n")
}

/// One border line: `l` + 7 * ("────" + `m`, last one `r` instead), wrapped
/// in a single [`crate::tooltip::C_EMPTY`]-coloured span — clock.sh's
/// `border()`.
fn border_row(l: &str, m: &str, r: &str) -> String {
    let mut s = String::from(l);
    for i in 0..7 {
        s.push_str("\u{2500}\u{2500}\u{2500}\u{2500}");
        s.push_str(if i < 6 { m } else { r });
    }
    format!(
        "<span foreground=\"{}\">{s}</span>",
        crate::tooltip::C_EMPTY
    )
}

/// One day-number line, read off `cal -m`'s fixed 3-byte columns (`cal`'s
/// output is pure ASCII, so byte slicing is safe) — clock.sh's `cells()`.
/// Reading the exact column rather than a digit regex is what clock.sh's
/// own comment calls out as the reason a two-digit day (e.g. the "2" in
/// "12") is never mistaken for a different single-digit day.
fn cell_row(line: &str, colour: &str, bold: bool, today: u32) -> String {
    use crate::tooltip::C_TITLE;

    let bytes = line.as_bytes();
    let mut out = String::from("\u{2502}"); // │
    for i in 0..7 {
        let start = i * 3;
        let raw = if start + 2 <= bytes.len() {
            std::str::from_utf8(&bytes[start..start + 2]).unwrap_or("  ")
        } else {
            "  "
        };
        let trimmed = raw.trim();
        // Two literal spaces for a blank cell — not a span-wrapped raw
        // value — so the self-check's stripped-tag byte width (52 bytes:
        // 8 × 3-byte "│" + 28 single-byte cell bytes) matches a day cell
        // exactly; a span with no visible characters would still measure
        // wrong once its tags are stripped for that check.
        let cell = if trimmed.is_empty() {
            "  ".to_string()
        } else {
            let day: u32 = trimmed.parse().unwrap_or(0);
            let is_today = bold && day == today;
            let (c, weight) = if is_today {
                (C_TITLE, " font_weight=\"bold\"")
            } else {
                (colour, "")
            };
            format!("<span foreground=\"{c}\"{weight}>{raw}</span>")
        };
        out.push(' ');
        out.push_str(&cell);
        out.push(' ');
        out.push('\u{2502}');
    }
    format!(
        "<span foreground=\"{}\">{out}</span>",
        crate::tooltip::C_EMPTY
    )
}

/// clock.sh:195-208 (`--date` branch) — title (`%B %Y`), the bordered month
/// table with today highlighted, footer line. `date_tip` ironvar for the
/// click popup genconfig.rs wires up.
pub async fn refresh_date_tip(vars: &mut Vars) {
    use crate::tooltip::{mono, set_titled};

    let title_text = format_local("%B %Y");
    let day: u32 = format_local("%-d").trim().parse().unwrap_or(1);
    let grid = cal_grid(day, None).await;

    let mut tip = String::new();
    tip.push_str(&mono(&format!("<span size='115%'>{grid}</span>")));

    // T13: the old hardcoded "click for the calendar" footer here predated
    // T-hover and was stale (there is no more click-to-open) — set_tip's own
    // HINTS-driven footer replaces it with the real gesture. T15: that
    // gesture is left-click now (middle-click retired bar-wide).
    set_titled(vars, "date_tip", &title_text, tip);
}

fn format_local(fmt: &str) -> String {
    // SAFETY: libc::time never fails (no error return per POSIX).
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    format_epoch(t, fmt)
}

/// Same as `format_local` but for an arbitrary epoch, not just "now" — used
/// by pomo.rs's log line, which formats a phase's start/end times rather
/// than the current moment.
pub(crate) fn format_epoch(t: libc::time_t, fmt: &str) -> String {
    // SAFETY: `t` is a caller-supplied time_t; `tm` is a zeroed,
    // correctly-sized out-param filled in place by localtime_r; `cfmt` is a
    // NUL-terminated CString kept alive through the strftime call; `buf` is
    // sized generously (128 bytes) for the short formats this crate uses,
    // and strftime's return value (bytes written) is used to truncate it —
    // never trusted as a length beyond what strftime itself reports.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let cfmt = std::ffi::CString::new(fmt).expect("format has no NUL bytes");
        let mut buf = vec![0u8; 128];
        let n = libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            cfmt.as_ptr(),
            &tm,
        );
        buf.truncate(n);
        String::from_utf8_lossy(&buf).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vars::Vars;

    // ---- tz_offset(): 6 cases, copied verbatim from clock.sh:132-137.
    #[test]
    fn tz_offset_matches_clock_sh_selftest_table() {
        assert_eq!(tz_offset("+0400", "+0400"), "same");
        assert_eq!(tz_offset("+0100", "+0400"), "-3h");
        assert_eq!(tz_offset("+0900", "+0400"), "+5h");
        assert_eq!(tz_offset("+0545", "+0400"), "+1h45");
        assert_eq!(tz_offset("-0500", "+0400"), "-9h");
        assert_eq!(tz_offset("+0000", "-0330"), "+3h30");
    }

    // ---- tz_label(): clock.sh:139-140.
    #[test]
    fn tz_label_strips_country_or_overrides_it() {
        assert_eq!(tz_label("America/New_York"), "New York");
        assert_eq!(tz_label("UTC"), "UTC");
        assert_eq!(tz_label("Indian/Mauritius"), "Port Louis");
    }

    // ---- ctr(): clock.sh:142.
    #[test]
    fn ctr_centres_with_the_smaller_pad_on_the_left() {
        assert_eq!(ctr("ab", 6), "  ab  ");
        assert_eq!(ctr("abc", 6), " abc  ");
    }

    // ---- world column widths: clock.sh:144-148 — the enlarged time row and
    // the label row under it must span the same pixel width, or the three
    // world columns stop lining up.
    #[test]
    fn world_column_widths_match_at_time_scale() {
        const COL_W: u32 = 15;
        const COL_GAP: u32 = 3;
        const T_W: u32 = 10;
        const T_GAP: u32 = 2;
        const TIME_SCALE: u32 = 150;
        const WORLD_COLS: u32 = 3;
        let time_row_width = (T_W * WORLD_COLS + T_GAP * (WORLD_COLS - 1)) * TIME_SCALE;
        let label_row_width = (COL_W * WORLD_COLS + COL_GAP * (WORLD_COLS - 1)) * 100;
        assert_eq!(time_row_width, label_row_width);
    }

    // ---- day_wrap_suffix(): clock.sh:249-251.
    #[test]
    fn day_wrap_suffix_handles_the_new_year_wrap() {
        assert_eq!(day_wrap_suffix(101, 100), " +1d");
        assert_eq!(day_wrap_suffix(1, 365), " +1d"); // wraps at New Year's
        assert_eq!(day_wrap_suffix(100, 101), " -1d");
    }

    // ---- cal_grid_render(): clock.sh:151-173, against real captured
    // `cal -m` fixtures (not forked in the test itself — cal_grid_render is
    // the pure half `cal_grid` was split from for exactly this reason).
    const AUG_2026: &str = "     August 2026    \nMo Tu We Th Fr Sa Su\n                1  2\n 3  4  5  6  7  8  9\n10 11 12 13 14 15 16\n17 18 19 20 21 22 23\n24 25 26 27 28 29 30\n31                  \n";
    const FEB_2026: &str = "    February 2026   \nMo Tu We Th Fr Sa Su\n                   1\n 2  3  4  5  6  7  8\n 9 10 11 12 13 14 15\n16 17 18 19 20 21 22\n23 24 25 26 27 28   \n                    \n";

    #[test]
    fn cal_grid_marks_exactly_one_day() {
        // August 2026 starts on a Saturday, so Monday-first puts the 1st in
        // column 6.
        let c = cal_grid_render(AUG_2026, 12);
        assert_eq!(c.matches(crate::tooltip::C_TITLE).count(), 1);
        assert!(c.contains("font_weight=\"bold\">12</span>"));
    }

    #[test]
    fn cal_grid_does_not_confuse_a_two_digit_day_with_its_last_digit() {
        // The "2" in 12/21/25 must not be mistaken for the 2nd.
        let c = cal_grid_render(AUG_2026, 2);
        assert_eq!(c.matches(crate::tooltip::C_TITLE).count(), 1);
    }

    #[test]
    fn cal_grid_opens_and_closes_with_rounded_borders() {
        let c = cal_grid_render(AUG_2026, 12);
        let first = c.lines().next().unwrap();
        let last = c.lines().last().unwrap();
        assert!(first.starts_with(&format!(
            "<span foreground=\"{}\">\u{256d}",
            crate::tooltip::C_EMPTY
        )));
        assert!(last.ends_with("\u{256f}</span>"));
    }

    #[test]
    fn cal_grid_row_count_matches_the_week_count() {
        // 6 week rows in August 2026, plus top/header/separator/bottom.
        assert_eq!(cal_grid_render(AUG_2026, 12).lines().count(), 10);
        // February 2026 is 5 rows — the trailing all-blank guard row `cal`
        // emits must not be counted.
        assert_eq!(cal_grid_render(FEB_2026, 1).lines().count(), 9);
    }

    #[test]
    fn cal_grid_lines_are_all_36_cells_wide() {
        // Byte width after stripping span tags: 8 × 3-byte "│" (24) + 28
        // single-byte cell bytes (52) for a day row, 36 × 3-byte box glyphs
        // (108) for a border row — counted in bytes, not characters, so the
        // check does not depend on locale.
        fn strip_tags(s: &str) -> usize {
            let mut out = 0;
            let mut in_tag = false;
            for ch in s.chars() {
                match ch {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    _ if !in_tag => out += ch.len_utf8(),
                    _ => {}
                }
            }
            out
        }
        let mut widths: Vec<usize> = cal_grid_render(AUG_2026, 12)
            .lines()
            .map(strip_tags)
            .collect();
        widths.sort_unstable();
        widths.dedup();
        assert_eq!(widths, vec![52, 108]);
    }

    // ---- build_tip shape: real forks (`date`/`cal`), matching clock.sh's
    // own self-check forking a real `cal`.
    #[tokio::test]
    async fn refresh_clock_tip_has_local_and_world_sections() {
        let mut vars = Vars::new();
        refresh_clock_tip(&mut vars).await;
        let (k, tip) = vars
            .peek_dirty()
            .expect("refresh_clock_tip must set clk_tip");
        assert_eq!(k, "clk_tip");
        assert!(tip.contains("Local"));
        assert!(tip.contains("World"));
    }

    #[tokio::test]
    async fn refresh_date_tip_has_a_calendar_grid() {
        let mut vars = Vars::new();
        refresh_date_tip(&mut vars).await;
        let (k, tip) = vars
            .peek_dirty()
            .expect("refresh_date_tip must set date_tip");
        assert_eq!(k, "date_tip");
        // T-popup-sep: the "click: open calendar app" footer is no longer
        // appended to the ironvar body — genconfig.rs's `popup()` renders it
        // as a static widget instead, see that module's own test.
        assert!(tip.contains('\u{2502}')); // the grid's own │ border
    }
}
