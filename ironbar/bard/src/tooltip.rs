//! Seed of the tooltip/popup markup vocabulary IRONBAR.md assigns in full to
//! T7's `tooltip.rs` (meters, sections, per-process tables, replacing
//! `tooltip.sh` and claudebar's private copy). T2 needs only what the
//! workspace and window popups use: escaping, row inset, and the per-tag
//! window list — ported from src/waybar/scripts/tooltip.sh and
//! workspace.sh's `winrows()`.

use crate::mango::Win;

/// U+00A0, not a plain space — Pango's width request can drop trailing
/// plain spaces, which would silently undo a right-margin fix on whichever
/// row happens to be last. NBSP survives that and renders identically.
const NBSP: char = '\u{a0}';

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

fn row(body: &str) -> String {
    format!("{NBSP}{NBSP}{NBSP}{body}{NBSP}{NBSP}{NBSP}")
}

fn dim(text: &str) -> String {
    format!("{NBSP}{NBSP}{NBSP}<span foreground=\"#5c6370\">{text}</span>{NBSP}{NBSP}{NBSP}")
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
}
