//! Tooltip/popup markup vocabulary, ported from
//! src/ironbar/scripts/tooltip.sh. T2 took escaping, row inset and the
//! per-tag window list (workspace.sh's `winrows()`); T4 (audio.rs) adds the
//! meter/section/grade vocabulary its volume popup needs; T5 (power.rs) adds
//! the compact key/value row and duration formatters its battery popup
//! needs. Full parity with tooltip.sh (per-process tables, claudebar's
//! private copy) is still T7's job — see IRONBAR.md "Tooltips and popups".

use std::fmt;
use std::sync::{LazyLock, RwLock};

use crate::mango::Win;
use crate::vars::Vars;

/// U+00A0, not a plain space — Pango's width request can drop trailing
/// plain spaces, which would silently undo a right-margin fix on whichever
/// row happens to be last. NBSP survives that and renders identically.
pub(crate) const NBSP: char = '\u{a0}';

/// tooltip.sh:31 — Google Sans Flex's proportional figures drift a column
/// padded with `%3s`/`%3d` by a few pixels per row; anything that has to
/// line up column-for-column names this family explicitly instead.
const F_MONO: &str = "JetBrainsMono Nerd Font";

// -------------------------------------------------------------- meters

/// Index into [`PALETTE`]. `Copy` + `Display`, so a `C_*` const still drops
/// straight into a `format!("{C_TITLE}")` capture exactly like the `&str`
/// literals it replaces — the only call sites that need touching are the
/// few that pass a colour on as a typed parameter (`grade`/`bar`/`dot`/
/// `cell_row`). Slots 4-6 (good/warn/bad) are never written by
/// [`reload_palette`] — see its own comment for why those three stay fixed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ink(usize);

impl fmt::Display for Ink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&PALETTE.read().unwrap_or_else(|e| e.into_inner())[self.0])
    }
}

/// One-dark defaults. Slots 0-3 and 7 are overwritten by [`reload_palette`]
/// once matugen's `colors.json` exists; a fresh checkout with no matugen run
/// yet renders identically to before this change. Slots 4-6 are the
/// permanent good/warn/bad values — semantic colours, exempt from matugen
/// for the same reason `style.css`'s `@define-color ok`/`caution` are: a
/// meter that means "fine" must not turn wallpaper-orange.
const ONE_DARK: [&str; 8] = [
    "#61afef", // 0 C_TITLE
    "#3e4451", // 1 C_RULE
    "#abb2bf", // 2 C_LABEL
    "#5c6370", // 3 C_DIM
    "#98c379", // 4 C_GOOD (fixed)
    "#e5c07b", // 5 C_WARN (fixed)
    "#e06c75", // 6 C_BAD (fixed)
    "#3e4451", // 7 C_EMPTY
];

static PALETTE: LazyLock<RwLock<[String; 8]>> =
    LazyLock::new(|| RwLock::new(ONE_DARK.map(String::from)));

pub const C_TITLE: Ink = Ink(0);
const C_RULE: Ink = Ink(1);
/// pub: clock.rs's `cal_grid` needs this directly for the calendar's day
/// cells, same reason `C_DIM` is already public for docker.rs.
pub const C_LABEL: Ink = Ink(2);
/// pub: docker.rs needs this directly for its per-row image-name span
/// (docker.sh:148 uses `$C_DIM` from tooltip.sh the same way), not just
/// through a helper defined in this module.
pub const C_DIM: Ink = Ink(3);
pub const C_GOOD: Ink = Ink(4);
pub const C_WARN: Ink = Ink(5);
pub const C_BAD: Ink = Ink(6);
pub const C_EMPTY: Ink = Ink(7);

/// `(PALETTE index, colors.json key)` for every ink matugen actually tracks —
/// slots 4-6 (good/warn/bad) are deliberately absent, see [`ONE_DARK`].
const MATUGEN_KEYS: [(usize, &str); 5] = [
    (0, "primary"),
    (1, "outline_variant"),
    (2, "on_surface"),
    (3, "on_surface_variant"),
    (7, "surface_container_highest"),
];

/// Re-reads matugen's `colors.json` and swaps the live palette. Called from
/// `main.rs::dispatch_refresh` on the `colors` topic, which `switchwall.sh`
/// sends after every wallpaper/mode switch (T6b's `darkmode` topic still
/// exists and still runs — `colors` falls through to the same full-resync
/// `_` arm, it just also does this first). Missing file or bad JSON leaves
/// the current palette untouched rather than blanking it — a stale palette
/// beats no palette.
pub fn reload_palette() {
    // Same XDG_STATE_HOME resolution power.rs::Power::new() already uses.
    let state_home = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
        });
    let path = state_home.join("mango/generated/colors.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let mut palette = PALETTE.write().unwrap_or_else(|e| e.into_inner());
    for (slot, key) in MATUGEN_KEYS {
        if let Some(hex) = json.get(key).and_then(|v| v.as_str()) {
            palette[slot] = hex.to_string();
        }
    }
}

/// tooltip.sh:67 — bold blue title line.
pub fn title(text: &str) -> String {
    format!("<span font_weight=\"bold\" foreground=\"{C_TITLE}\">{text}</span>\n")
}

/// tooltip.sh:78-85 — N copies of `─`, default 30 (callers here always pass
/// an explicit width sized to their own content, same as the shell version).
pub fn rule(cells: usize) -> String {
    let dashes: String = std::iter::repeat_n('─', cells).collect();
    format!("<span foreground=\"{C_RULE}\">{dashes}</span>\n")
}

/// tooltip.sh:89 — section header: icon, two spaces, label, NBSP margin.
pub fn sect(icon: &str, label: &str) -> String {
    format!("\n{NBSP}{NBSP}<span foreground=\"{C_LABEL}\">{icon}  {label}</span>{NBSP}{NBSP}\n")
}

pub fn good(text: &str) -> String {
    format!("<span foreground=\"{C_GOOD}\">{text}</span>")
}
pub fn bad(text: &str) -> String {
    format!("<span foreground=\"{C_BAD}\">{text}</span>")
}
/// T19: sibling of `good`/`bad`, added for net.rs's three detail popups —
/// net.sh's own `warn()` (tooltip.sh) is used in five places across
/// `sec_emit`/`wifi_emit` (route conflicts, security warnings) that this
/// port needs and had no counterpart for until now.
pub fn warn(text: &str) -> String {
    format!("<span foreground=\"{C_WARN}\">{text}</span>")
}

/// tooltip.sh:106-109 — good/warn/bad colour for a percentage against two
/// ascending thresholds.
pub fn grade(pct: i64, warn_at: i64, bad_at: i64) -> Ink {
    if pct >= bad_at {
        C_BAD
    } else if pct >= warn_at {
        C_WARN
    } else {
        C_GOOD
    }
}

/// tooltip.sh:117-124 — 20-cell (default) capsule: `n` NBSPs on a
/// `colour`-background span, then the remainder on an empty-track span, both
/// `size="55%"` so the run reads as a thin bar. `n = round(pct*cells/100)`,
/// clamped to `0..=cells`.
pub fn bar(pct: i64, colour: Ink, cells: usize) -> String {
    let n = ((pct * cells as i64 + 50) / 100).clamp(0, cells as i64) as usize;
    let filled: String = std::iter::repeat_n(NBSP, n).collect();
    let empty: String = std::iter::repeat_n(NBSP, cells - n).collect();
    format!(
        "<span size=\"55%\" background=\"{colour}\">{filled}</span>\
         <span size=\"55%\" background=\"{C_EMPTY}\">{empty}</span>"
    )
}

/// tooltip.sh:65 — every icon-bearing bar/tooltip glyph wraps identically.
///
/// T15: `size="115%"` dropped to `100%` — every glyph through this helper
/// (volume, netsec, hotspot, wifi-off, eth-disconnected) rendered visibly
/// larger than the 15px text beside it on the bar; nothing here needs to be
/// bigger than its neighbours. `rise` moves to `0` to match — it was only
/// ever compensating for the enlarged glyph sitting low against the
/// baseline, see `ICO_RISE`'s own comment for the same correction on
/// [`barico_label`].
pub fn barico(icon: char) -> String {
    format!("<span size=\"100%\" rise=\"0\">{icon}</span>")
}

/// Pango `rise` is 1/1024 pt, `letter_spacing` likewise; at 96 dpi 1px = 768
/// units. Both are calibration knobs, retuned against a live screenshot —
/// not derived constants.
///
/// `barico()`'s old `rise="-1200"` was tuned for Material Symbols Rounded,
/// the font T8b removed from the whole bar, then retuned once for a 115%
/// JetBrainsMono Nerd Font glyph. T15 drops the glyph back to 100% (its own
/// comment), which needs no rise correction at all — `0` is the starting
/// point for the next screenshot-driven retune, not a derived value.
/// T16: `ICO_GAP` doubled, 2048 -> 4096 (~2.7px -> ~5.3px at 96 dpi). Every
/// `barico_label()` caller (cpu, memory, docker, battery, wifi, eth, pomo,
/// claude) reported the number sitting almost against the glyph — but the
/// real cause (found at T18) was the icon font's own ink spilling up to
/// 8.3px past its advance box (style.css's T18 comment on the icon-pill
/// rule has the measurements), which this gap was papering over one glyph
/// at a time. With the font swapped to the Propo variant (zero spill),
/// 4096 reads as a visibly wider gap than any neighbouring pill uses —
/// dropped back to T16's own starting point, 2048. Retune from a live
/// screenshot if this over- or under-shoots — each call site also adds its
/// own literal space on top of this gap, so the visible gap is never this
/// value alone.
const ICO_RISE: i32 = 0;
const ICO_GAP: i32 = 2048;

/// Icon+label variant of [`barico`] — every pill whose bar text is an icon
/// followed immediately by a value (cpu/memory/docker's percent or count,
/// eth's IP, wifi's signal percent, battery's charge percent, pomodoro's
/// countdown). `barico()` itself is untouched (see its own doc comment) so
/// icon-only pills never move.
pub fn barico_label(icon: char) -> String {
    format!("<span size=\"100%\" rise=\"{ICO_RISE}\" letter_spacing=\"{ICO_GAP}\">{icon}</span>")
}

/// Escapes `&`, `<`, `>` for embedding in Pango markup, in that order (so
/// entities are never double-escaped). Matches tooltip.sh's `esc()` /
/// workspace.sh's inline `winrows()` escaper.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

pub fn row(body: &str) -> String {
    format!("{NBSP}{NBSP}{NBSP}{body}{NBSP}{NBSP}{NBSP}")
}

pub fn dim(text: &str) -> String {
    format!("{NBSP}{NBSP}{NBSP}<span foreground=\"{C_DIM}\">{text}</span>{NBSP}{NBSP}{NBSP}")
}

/// Gesture hints for a popup's footer. These strings are the static GTK
/// `tooltip` text that used to live on the same modules in genconfig.rs —
/// deleted at T13 because a tooltip and a popup fire on the same hover and
/// compete for the same space. Keyed by tip ironvar, so the list stays in
/// one place instead of scattered across nine collectors. `clk_tip` is
/// deliberately absent: its old tooltip said only "hover for world times",
/// which the popup now demonstrates. Workspace-pill tips are also absent —
/// their popup body already reads as the hint (mango.rs's `apply()`).
///
/// T15: middle-click retired bar-wide (genconfig.rs) — every "middle-click"
/// string below is rewritten for the gesture its action actually moved to.
/// `hotspot_tip` is new: hotspot's static `tooltip` moved here when it
/// joined the hover-opens-the-popup group (genconfig.rs's `hotspot_module`
/// doc comment).
///
/// T19: `wifi_tip`/`eth_tip`/`sec_tip` join the same way — their old static
/// `tooltip` strings moved here now that all three have a hover popup (see
/// net.rs's three `refresh_*_tip` functions).
///
/// T-popup-sep: no longer appended into the tip body by [`set_tip`] — a
/// pango rule of `─` characters never actually reached the popup's real
/// width. `genconfig.rs` reads this table directly and renders each hint as
/// its own `popup-hint` label, below a real `popup-sep` box widget (a GTK
/// box painted 100% wide by CSS) — see that module's `popup_widgets()`.
/// `pub(crate)`: genconfig.rs is a sibling module needing read access.
pub(crate) const HINTS: &[(&str, &str)] = &[
    ("cpu_tip", "click: btop"),
    ("mem_tip", "click: btop"),
    ("date_tip", "click: open calendar app"),
    ("pomo_tip", "click: start/pause · right-click: mute"),
    (
        "vol_tip",
        "click: mixer · scroll: volume · right-click: pavucontrol",
    ),
    (
        "bat_tip",
        "click: toggle power mode · right-click: powertop",
    ),
    ("hotspot_tip", "click: menu · right-click: toggle"),
    (
        "remote_tip",
        "click: toggle VNC + KDE Connect · right-click: pull next tag",
    ),
    ("inhibit_tip", "click: toggle keep-awake"),
    ("docker_tip", "right-click: menu"),
    ("claude_tip", "right-click: settings"),
    ("kp_tip", "click: show/hide KeePassXC"),
    ("au_tip", "click: run arch-update"),
    ("music_tip", "click: play/pause"),
    (
        "wifi_tip",
        "click: pick a network · right-click: edit connections",
    ),
    (
        "eth_tip",
        "click: toggle adapter · right-click: edit connections",
    ),
    ("sec_tip", "click: details · right-click: edit connections"),
];

/// `vars.set` for a popup tip. `body` is expected trailing-newline-trimmed,
/// same convention every tip builder already follows (cpu.rs's `build_tip`,
/// claude.rs's own `trim_end_matches('\n')`). No longer appends a HINTS
/// footer (T-popup-sep) — the footer is now a static widget in the popup's
/// own config, built once by `genconfig.rs` from the same [`HINTS`] table,
/// not re-appended into the ironvar body on every refresh.
pub fn set_tip(vars: &mut Vars, key: &str, body: String) {
    vars.set(key, body);
}

/// Keys whose popup carries a title widget above the body. Every collector
/// in this list used to open its `tip` string with `title(...)` (and a
/// `rule(N)` under it); T-popup-vert moved both out of the body — see
/// `set_titled`'s own doc comment for why.
///
/// `pub(crate)`: genconfig.rs reads this directly, same as [`HINTS`].
pub(crate) const TITLED: &[&str] = &[
    "cpu_tip",
    "mem_tip",
    "docker_tip",
    "bat_tip",
    "claude_tip",
    "vol_tip",
    "hotspot_tip",
    "remote_tip",
    "inhibit_tip",
    "clk_tip",
    "date_tip",
    "wifi_tip",
    "eth_tip",
    "sec_tip",
    "kp_tip",
    "au_tip",
    "music_tip",
];

/// `vars.set` for a popup that carries a title. Splits what a `tip` string
/// used to hold as its own first two lines (`title(text)` + `rule(N)`) into
/// two ironvars — `<key>_title` and `<key>` — that `genconfig.rs::popup()`
/// renders as two separate widgets either side of a real `box.popup-sep`.
///
/// T-popup-vert: the old in-body `rule(N)` was a pango run of `─`
/// characters sized by a hand-picked cell count per caller (13 different
/// magic numbers across cpu/memory/docker/.../net) that never actually
/// matched the popup's real width — see `genconfig.rs::popup()`'s own doc
/// comment for the GTK-box replacement. `title_text` is plain text; this
/// wraps it in the same bold-blue span [`title`] always used, trimmed of
/// its trailing `\n` since the title is now a whole label on its own, not
/// a body line other lines get joined under.
pub fn set_titled(vars: &mut Vars, key: &str, title_text: &str, body: String) {
    let title_key = format!("{key}_title");
    vars.set(
        &title_key,
        title(title_text).trim_end_matches('\n').to_string(),
    );
    vars.set(key, body);
}

/// T28: CSS class for a horizontal gauge fill, one per 5 percentage points
/// (`<prefix>0`..`<prefix>100`) — shared by cpu/memory/battery so style.css
/// needs the 21-step `linear-gradient` ramp written once per prefix, not
/// once per pill. Rounds down, not to nearest: a `p85` gauge fills to the
/// 85% stop exactly, never past the real value.
///
/// T29: takes a `prefix` (`"p"`/`"cl"`/`"ml"`) instead of hardcoding `p` —
/// `cpu` and `memory` now push their level class onto `sysload`'s single
/// shared node (see `sysload_module()`'s own doc comment for why they
/// can't be separate top-level modules any more), so `p45`/`p45` from both
/// would be indistinguishable and, worse, identical values never re-fire a
/// dirty `style add_class`/`remove_class` pair — the daemon would think
/// memory's `p45` was already applied by cpu. Distinct prefixes keep the
/// two independent. `battery` keeps the bare `p` prefix — it is still its
/// own module, no collision possible.
pub fn level_class(prefix: &str, pct: i64) -> String {
    format!("{prefix}{}", (pct.clamp(0, 100) / 5) * 5)
}

/// docker.sh:50-57 `dot()`: a colour-graded status dot for `class` in
/// `{"good","warn","bad"}`; anything else (docker.sh's 4th case) gets the
/// hollow ring instead of the filled dot.
pub fn dot(class: &str) -> String {
    let (glyph, colour) = match class {
        "good" => ('●', C_GOOD),
        "warn" => ('●', C_WARN),
        "bad" => ('●', C_BAD),
        _ => ('○', C_DIM),
    };
    format!("<span foreground=\"{colour}\">{glyph}</span>")
}

/// docker.sh:66 `projhdr()`: an unlabeled compose-project header line,
/// deliberately icon-less — every icon on this bar already means something
/// specific, a project name doesn't need one to read as a header.
pub fn projhdr(name: &str) -> String {
    format!("\n<span foreground=\"{C_LABEL}\">  {name}</span>\n")
}

/// tooltip.sh:36 — wraps the columnar part of a row (a meter and its
/// number) in the mono family, leaving trailing prose in the proportional
/// face. No trailing newline — always used inline within a `kv`/`kvsub` row.
pub fn mono(text: &str) -> String {
    format!("<span font_family=\"{F_MONO}\">{text}</span>")
}

/// tooltip.sh:99 — compact key/value row: an 8-cell label column in
/// [`F_MONO`] (a fixed-width label needs a fixed-width font, same reasoning
/// as [`mono`]), then the value as-is. Self-terminates with `\n`, unlike
/// [`row`]/[`dim`] — battery.rs's tooltip is a straight-line sequence of
/// these with no per-caller join needed.
pub fn kv(label: &str, value: &str) -> String {
    format!("{NBSP}{NBSP}{NBSP}<span font_family=\"{F_MONO}\">{label:<8}</span>{value}{NBSP}{NBSP}{NBSP}\n")
}

/// tooltip.sh:100 — a second, label-less row that lines up under [`kv`]'s
/// value column: an 8-space run in the same [`F_MONO`] span, so it measures
/// identically to an 8-character label instead of drifting in the
/// proportional face.
pub fn kvsub(value: &str) -> String {
    format!(
        "{NBSP}{NBSP}{NBSP}<span font_family=\"{F_MONO}\">{blank:<8}</span><span foreground=\"{C_DIM}\">{value}</span>{NBSP}{NBSP}{NBSP}\n",
        blank = ""
    )
}

/// tooltip.sh:174-179 — seconds to "2h 14m" / "14m" / "48s".
pub fn hdur(secs: i64) -> String {
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// tooltip.sh:156-163 — KiB to a human string (`memory.sh`'s RAM/swap rows).
/// `0` is a real value here (an empty swap file's used column), so it gets
/// its own literal case rather than falling out of the loop as `"0.0 KiB"`.
pub fn hkib(kib: i64) -> String {
    if kib == 0 {
        return "none".to_string();
    }
    let units = ["KiB", "MiB", "GiB", "TiB"];
    let mut k = kib as f64;
    let mut i = 0;
    while k >= 1024.0 && i < 3 {
        k /= 1024.0;
        i += 1;
    }
    if k >= 100.0 {
        format!("{k:.0} {}", units[i])
    } else {
        format!("{k:.1} {}", units[i])
    }
}

/// tooltip.sh:165-171 — a raw count to a human string (`memory.sh`'s
/// page-fault rates/totals, `cpu.sh`'s process counts). The first bucket
/// prints with no unit suffix at all — not even an empty one — matching the
/// shell's `"%d"` vs `"%.0f%s"`/`"%.1f%s"` format split.
pub fn hcount(n: i64) -> String {
    let units = ["", "k", "M", "G"];
    let mut n = n as f64;
    let mut i = 0;
    while n >= 1000.0 && i < 3 {
        n /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{}", n as i64)
    } else if n >= 100.0 {
        format!("{n:.0}{}", units[i])
    } else {
        format!("{n:.1}{}", units[i])
    }
}

/// tooltip.sh's `human()` — a byte rate to "1.2 MB/s"/"340 kB/s"/"0 B/s".
/// T19: net.rs's throughput rows. Exact port including the shell's own
/// `1024` divisor for a nominally-decimal unit table (`split("B kB MB GB",
/// u, " ")`, `while (b >= 1024...) b /= 1024`) — not corrected to a true
/// 1000-based SI scale here, to stay an honest port rather than a
/// re-derivation. `i == 0` here is awk's `i == 1` (1-indexed there, 0-
/// indexed here): the first bucket, `B`, never gets a decimal point even
/// below 100 — a "12.0 B/s" reads as false precision tooltip.sh's own
/// author rejected.
pub fn human(bytes_per_sec: i64) -> String {
    let units = ["B", "kB", "MB", "GB"];
    let mut b = bytes_per_sec as f64;
    let mut i = 0;
    while b >= 1024.0 && i < 3 {
        b /= 1024.0;
        i += 1;
    }
    if b >= 100.0 || i == 0 {
        format!("{b:.0} {}/s", units[i])
    } else {
        format!("{b:.1} {}/s", units[i])
    }
}

const HEATBAR_GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// tooltip.sh:134-145 — one cell per value, each graded and coloured by its
/// own reading rather than the series' average, so a single spike or trough
/// is never hidden by a flat overall colour. `gap` is the 1-based index of
/// the cell before which a literal space is inserted (0 = no gap) — T7a's
/// cpu.rs uses this to part P-cores from E-cores in `core_labels`' row;
/// power.rs's history heatbar has no such split and passes 0.
pub fn heatbar(pcts: &[i64], warn_at: i64, bad_at: i64, gap: usize) -> String {
    let mut out = String::new();
    for (i, &p) in pcts.iter().enumerate() {
        if gap != 0 && i + 1 == gap {
            out.push(' ');
        }
        let k = ((p as f64 / 12.5) as i64 + 1).clamp(1, 8) as usize;
        let colour = grade(p, warn_at, bad_at);
        out.push_str(&format!(
            "<span foreground=\"{colour}\">{}</span>",
            HEATBAR_GLYPHS[k - 1]
        ));
    }
    out
}

/// One markup row per window: mark + appid (monospace, padded to the widest
/// *raw* appid in this list, left-justified) + two spaces + dim title.
/// Padding precedes escaping — escaping first would misalign every row
/// holding a `&` or `<`, since entities lengthen the string after the width
/// was measured (workspace.sh:71-72).
fn winrows(wins: &[Win]) -> Vec<String> {
    let width = wins
        .iter()
        .map(|w| w.appid.chars().count())
        .max()
        .unwrap_or(0);
    wins.iter()
        .map(|w| {
            let padded = format!("{:<width$}", w.appid, width = width);
            format!(
                "<span font_family=\"JetBrainsMono Nerd Font\">{} {}</span>  <span foreground=\"{C_DIM}\">{}</span>",
                w.mark,
                esc(&padded),
                esc(w.title),
            )
        })
        .collect()
}

/// Full popup body for a workspace pill: one row per window, or a single
/// dim "empty" row when the tag has none. Port of workspace.sh's `$TIP`
/// build (module section, lines 210-226) minus the "Tag N" title and
/// "Windows" section header it had already dropped — chrome above two
/// lines of content.
pub fn window_list(wins: &[Win]) -> String {
    if wins.is_empty() {
        return dim("empty");
    }
    winrows(wins)
        .iter()
        .map(|r| row(r))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_class_rounds_down_to_the_nearest_5_and_clamps() {
        assert_eq!(level_class("p", 0), "p0");
        assert_eq!(level_class("p", 4), "p0");
        assert_eq!(level_class("p", 5), "p5");
        assert_eq!(level_class("p", 87), "p85");
        assert_eq!(level_class("p", 100), "p100");
        assert_eq!(level_class("p", 104), "p100");
        assert_eq!(level_class("p", -1), "p0");
    }

    #[test]
    fn level_class_prefix_keeps_cpu_and_memory_independent_on_sysload() {
        // T29: cpu/memory share one node (`sysload`) — distinct prefixes
        // are what keeps a same-percentage coincidence from looking like
        // "already applied" to the dirty-tracking flush.
        assert_eq!(level_class("cl", 45), "cl45");
        assert_eq!(level_class("ml", 45), "ml45");
        assert_ne!(level_class("cl", 45), level_class("ml", 45));
    }

    #[test]
    fn winrows_pads_to_widest_appid_and_widest_stays_unpadded() {
        let wins = vec![
            Win {
                mark: '*',
                appid: "kitty",
                title: "vim",
            },
            Win {
                mark: '_',
                appid: "firefox",
                title: "news",
            },
        ];
        let rows = winrows(&wins);
        assert!(rows[0].contains("* kitty  </span>"), "{}", rows[0]);
        assert!(rows[1].contains("_ firefox</span>"), "{}", rows[1]);
    }

    #[test]
    fn winrows_pads_before_escaping() {
        let wins = vec![
            Win {
                mark: '*',
                appid: "a&b",
                title: "tt",
            },
            Win {
                mark: '_',
                appid: "longer",
                title: "u",
            },
        ];
        let rows = winrows(&wins);
        assert!(rows[0].contains("a&amp;b   </span>"), "{}", rows[0]);
    }

    #[test]
    fn winrows_escapes_title() {
        let wins = vec![Win {
            mark: '*',
            appid: "kitty",
            title: "x & y",
        }];
        let rows = winrows(&wins);
        assert!(rows[0].ends_with("x &amp; y</span>"), "{}", rows[0]);
    }

    #[test]
    fn window_list_falls_back_to_dim_empty() {
        assert!(window_list(&[]).contains("empty"));
    }

    // ---------------------------------------------- meters, ported from
    // tooltip.sh's `tooltip-selftest` (tooltip.sh:225-261).

    fn bar_expected(filled: usize, empty: usize) -> String {
        let f: String = std::iter::repeat_n(NBSP, filled).collect();
        let e: String = std::iter::repeat_n(NBSP, empty).collect();
        format!(
            "<span size=\"55%\" background=\"{C_GOOD}\">{f}</span>\
             <span size=\"55%\" background=\"{C_EMPTY}\">{e}</span>"
        )
    }

    #[test]
    fn bar_50_pct_splits_evenly_at_default_width() {
        assert_eq!(bar(50, C_GOOD, 20), bar_expected(10, 10));
    }

    #[test]
    fn bar_0_pct_is_all_empty() {
        assert_eq!(bar(0, C_GOOD, 20), bar_expected(0, 20));
    }

    #[test]
    fn bar_clamps_above_100_pct() {
        assert_eq!(bar(999, C_GOOD, 20), bar_expected(20, 0));
    }

    #[test]
    fn bar_narrows_to_a_given_cell_count() {
        assert_eq!(bar(50, C_GOOD, 10), bar_expected(5, 5));
    }

    #[test]
    fn rule_width_matches_cell_count() {
        assert_eq!(rule(5).matches('─').count(), 5);
        assert_eq!(rule(90).matches('─').count(), 90);
    }

    #[test]
    fn grade_picks_good_warn_bad_at_thresholds() {
        assert_eq!(grade(10, 70, 90), C_GOOD);
        assert_eq!(grade(75, 70, 90), C_WARN);
        assert_eq!(grade(95, 70, 90), C_BAD);
    }

    #[test]
    fn row_and_dim_margins_match_on_both_ends() {
        let ind3: String = std::iter::repeat_n(NBSP, 3).collect();
        assert!(row("x").starts_with(&ind3));
        assert!(row("x").ends_with(&ind3));
        assert!(dim("x").starts_with(&ind3));
        assert!(dim("x").ends_with(&ind3));
    }

    #[test]
    fn sect_right_margin_is_two_nbsp() {
        let ind2: String = std::iter::repeat_n(NBSP, 2).collect();
        assert!(sect("i", "l").trim_end_matches('\n').ends_with(&ind2));
    }

    // ------------------------------------------- T5 additions, ported from
    // tooltip.sh's `tooltip-selftest` (tooltip.sh:250-251, 253).

    #[test]
    fn kv_label_and_kvsub_blank_column_measure_identically() {
        let ind3: String = std::iter::repeat_n(NBSP, 3).collect();
        assert_eq!(
            kv("Mode", "X"),
            format!("{ind3}<span font_family=\"{F_MONO}\">Mode    </span>X{ind3}\n")
        );
        assert_eq!(
            kvsub("Y"),
            format!(
                "{ind3}<span font_family=\"{F_MONO}\">        </span><span foreground=\"{C_DIM}\">Y</span>{ind3}\n"
            )
        );
    }

    #[test]
    fn mono_wraps_in_the_mono_font_with_no_trailing_newline() {
        assert_eq!(
            mono("x"),
            format!("<span font_family=\"{F_MONO}\">x</span>")
        );
    }

    #[test]
    fn hdur_formats_hours_minutes_and_seconds() {
        assert_eq!(hdur(8040), "2h 14m");
        assert_eq!(hdur(90), "1m 30s");
        assert_eq!(hdur(9), "9s");
    }

    #[test]
    fn heatbar_glyph_ramp_from_low_to_high() {
        let s = heatbar(&[0, 50, 100], 70, 90, 0);
        let ai = s.find('▁').unwrap();
        let bi = s.find('▅').unwrap();
        let ci = s.find('█').unwrap();
        assert!(ai < bi && bi < ci, "glyphs out of order: {s}");
    }

    // ------------------------------------------- T7a addition, ported from
    // tooltip.sh:238-239 (heatbar's optional gap-before-index argument).
    #[test]
    fn heatbar_gap_inserts_a_space_before_the_given_index() {
        assert!(heatbar(&[0, 0, 0], 70, 90, 3).contains("</span> <span"));
    }

    #[test]
    fn heatbar_does_not_gap_without_an_index() {
        assert!(!heatbar(&[0, 0, 0], 70, 90, 0).contains("</span> <span"));
    }

    // ------------------------------------------- T6b additions, ported from
    // docker.sh:50-57, 66.

    #[test]
    fn dot_grades_good_warn_bad_and_falls_back_to_hollow() {
        assert!(dot("good").contains('●'));
        assert!(dot("good").contains(&C_GOOD.to_string()));
        assert!(dot("warn").contains(&C_WARN.to_string()));
        assert!(dot("bad").contains(&C_BAD.to_string()));
        assert!(
            dot("dim").contains('○'),
            "unrecognized class must be hollow"
        );
    }

    #[test]
    fn projhdr_has_no_icon_unlike_sect() {
        let h = projhdr("myproject");
        assert!(h.contains("myproject"));
        assert!(h.starts_with('\n') && h.ends_with('\n'));
    }

    #[test]
    fn heatbar_colours_each_cell_by_its_own_value() {
        assert_eq!(
            heatbar(&[0, 75, 95], 70, 90, 0),
            format!(
                "<span foreground=\"{C_GOOD}\">▁</span><span foreground=\"{C_WARN}\">▇</span><span foreground=\"{C_BAD}\">█</span>"
            )
        );
    }

    // ------------------------------------------- T7a additions, ported from
    // tooltip.sh:148-171 (`human`/`hkib`/`hcount` — human's the net.rs T7b
    // gap; hkib/hcount are exercised here since cpu.rs/memory.rs both need
    // them for T7a's popups).

    #[test]
    fn hkib_matches_tooltip_sh_selftest() {
        assert_eq!(hkib(1_048_576), "1.0 GiB");
        assert_eq!(hkib(0), "none");
    }

    #[test]
    fn hcount_matches_tooltip_sh_selftest() {
        assert_eq!(hcount(1_234_567), "1.2M");
    }

    // ------------------------------------------- item 3 additions
    // (IRONBAR.md T-next): barico() vs barico_label() carry different
    // `rise` values, and only the label variant carries `letter_spacing`.

    #[test]
    fn barico_and_barico_label_differ_only_by_letter_spacing() {
        // T15: both dropped to a shared rise=0 once the 115% oversize that
        // motivated a separate rise correction was itself removed — see
        // ICO_RISE's own comment. The two helpers still aren't identical:
        // barico_label() alone carries letter_spacing (its own test below).
        assert!(barico('x').contains(&format!("rise=\"{ICO_RISE}\"")));
        assert!(barico_label('x').contains(&format!("rise=\"{ICO_RISE}\"")));
        assert_ne!(barico('x'), barico_label('x'));
    }

    #[test]
    fn only_barico_label_carries_letter_spacing() {
        assert!(!barico('x').contains("letter_spacing"));
        assert!(barico_label('x').contains(&format!("letter_spacing=\"{ICO_GAP}\"")));
    }

    // ---------------------------- T-popup-sep: set_tip is a plain passthrough
    // now — the HINTS footer moved to a static widget in genconfig.rs.

    #[test]
    fn set_tip_passes_the_body_through_unchanged() {
        let mut vars = Vars::new();
        set_tip(&mut vars, "cpu_tip", "body".to_string());
        assert_eq!(vars.peek_dirty(), Some(("cpu_tip", "body")));
    }

    #[test]
    fn set_tip_leaves_an_empty_body_empty() {
        let mut vars = Vars::new();
        set_tip(&mut vars, "docker_tip", String::new());
        assert_eq!(vars.peek_dirty(), Some(("docker_tip", "")));
    }

    #[test]
    fn hints_keys_match_the_expected_tip_set() {
        // A typo'd key here would silently drop a hint with no test failure
        // anywhere else — this pins the exact set so that can't happen.
        let mut keys: Vec<&str> = HINTS.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "au_tip",
                "bat_tip",
                "claude_tip",
                "cpu_tip",
                "date_tip",
                "docker_tip",
                "eth_tip",
                "hotspot_tip",
                "inhibit_tip",
                "kp_tip",
                "mem_tip",
                "music_tip",
                "pomo_tip",
                "remote_tip",
                "sec_tip",
                "vol_tip",
                "wifi_tip",
            ]
        );
    }

    #[test]
    fn titled_keys_match_the_expected_tip_set() {
        // Same guard as hints_keys_match_the_expected_tip_set, for TITLED.
        let mut keys: Vec<&str> = TITLED.to_vec();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "au_tip",
                "bat_tip",
                "claude_tip",
                "clk_tip",
                "cpu_tip",
                "date_tip",
                "docker_tip",
                "eth_tip",
                "hotspot_tip",
                "inhibit_tip",
                "kp_tip",
                "mem_tip",
                "music_tip",
                "remote_tip",
                "sec_tip",
                "vol_tip",
                "wifi_tip",
            ]
        );
    }

    #[test]
    fn set_titled_writes_body_and_a_trimmed_title_to_separate_keys() {
        let mut vars = Vars::new();
        set_titled(&mut vars, "cpu_tip", "CPU", "body".to_string());
        vars.ack("cpu_tip");
        vars.ack("cpu_tip_title");
        assert_eq!(vars.live_value("cpu_tip"), Some("body"));
        assert_eq!(
            vars.live_value("cpu_tip_title"),
            Some(title("CPU").trim_end_matches('\n'))
        );
    }

    // ------------------------------------------- T19 additions: warn() and
    // human(), net.rs's detail-popup gaps.

    #[test]
    fn warn_uses_the_warn_colour() {
        assert_eq!(warn("x"), format!("<span foreground=\"{C_WARN}\">x</span>"));
    }

    #[test]
    fn human_matches_tooltip_sh_selftest() {
        // Exact port of tooltip.sh's own 1024-divisor, decimal-unit-table
        // human() — see the function's own doc comment for why 1024 stays.
        assert_eq!(human(0), "0 B/s");
        assert_eq!(human(999), "999 B/s");
        assert_eq!(human(1024), "1.0 kB/s");
        assert_eq!(human(1_048_576), "1.0 MB/s");
        assert_eq!(human(104_857_600), "100 MB/s");
    }

    #[test]
    fn human_bytes_bucket_never_gets_a_decimal() {
        assert_eq!(human(12), "12 B/s");
    }
}
