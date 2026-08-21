//! Tooltip/popup markup vocabulary, ported from
//! src/waybar/scripts/tooltip.sh. T2 took escaping, row inset and the
//! per-tag window list (workspace.sh's `winrows()`); T4 (audio.rs) adds the
//! meter/section/grade vocabulary its volume popup needs. Full parity with
//! tooltip.sh (per-process tables, claudebar's private copy) is still T7's
//! job — see IRONBAR.md "Tooltips and popups".

use crate::mango::Win;

/// U+00A0, not a plain space — Pango's width request can drop trailing
/// plain spaces, which would silently undo a right-margin fix on whichever
/// row happens to be last. NBSP survives that and renders identically.
const NBSP: char = '\u{a0}';

// -------------------------------------------------------------- meters

// One-dark constants, hardcoded rather than matugen colors so tooltips stay
// legible against the fixed GTK tooltip background (tooltip.sh:13-23).
pub const C_TITLE: &str = "#61afef";
const C_RULE: &str = "#5c6370";
const C_LABEL: &str = "#abb2bf";
const C_DIM: &str = "#5c6370";
pub const C_GOOD: &str = "#98c379";
pub const C_WARN: &str = "#e5c07b";
pub const C_BAD: &str = "#e06c75";
pub const C_EMPTY: &str = "#3e4451";

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

/// tooltip.sh:106-109 — good/warn/bad colour for a percentage against two
/// ascending thresholds.
pub fn grade(pct: i64, warn_at: i64, bad_at: i64) -> &'static str {
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
pub fn bar(pct: i64, colour: &str, cells: usize) -> String {
    let n = ((pct * cells as i64 + 50) / 100).clamp(0, cells as i64) as usize;
    let filled: String = std::iter::repeat_n(NBSP, n).collect();
    let empty: String = std::iter::repeat_n(NBSP, cells - n).collect();
    format!(
        "<span size=\"55%\" background=\"{colour}\">{filled}</span>\
         <span size=\"55%\" background=\"{C_EMPTY}\">{empty}</span>"
    )
}

/// tooltip.sh:65 — every icon-bearing bar/tooltip glyph wraps identically.
pub fn barico(icon: char) -> String {
    format!("<span size=\"115%\" rise=\"-1200\">{icon}</span>")
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
                "<span font_family=\"JetBrainsMono Nerd Font\">{} {}</span>  <span foreground=\"#5c6370\">{}</span>",
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
}
