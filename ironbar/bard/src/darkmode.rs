//! Dark/light indicator glyph. Ports src/waybar/scripts/darkmode.sh's status
//! half (its `--toggle` half stays shell — it re-runs switchwall.sh's whole
//! matugen pipeline, not something to duplicate here).
//!
//! State lives entirely in gsettings
//! (`org.gnome.desktop.interface color-scheme`), written only by
//! switchwall.sh. This collector never watches for changes itself — per
//! IRONBAR.md T6b decision D3, switchwall.sh gains one `mango-bard refresh
//! darkmode` line next to its existing `pkill -SIGUSR2 waybar`
//! (switchwall.sh:120), rather than the daemon running its own `gsettings
//! monitor` child (measured ~22 ctxt-switches/min idle — worse than every
//! other watcher in this daemon, all of which measure 0/min).

use crate::vars::Vars;
use tokio::process::Command;

/// darkmode.sh:25 — shown while dark (click goes light).
const IC_LIGHT_MODE: char = '\u{e518}';
/// darkmode.sh:27 — shown while light (click goes dark). Icon shows the
/// action, not the state (darkmode.sh:22-23's own comment).
const IC_DARK_MODE: char = '\u{e51c}';

/// darkmode.sh:9-11 `is_dark()`: an unset or failed gsettings read counts as
/// dark — the shell's `!= *prefer-light*` test is true for anything that
/// isn't exactly `prefer-light`.
pub fn is_dark(color_scheme: &str) -> bool {
    !color_scheme.contains("prefer-light")
}

pub struct Darkmode;

impl Darkmode {
    pub fn new() -> Self {
        Self
    }

    /// The only thing that forks — one `gsettings get`, same as
    /// darkmode.sh's own single read.
    pub async fn refresh(&mut self, vars: &mut Vars) {
        let out = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
            .await;
        let value = match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
            Err(_) => String::new(),
        };
        let icon = if is_dark(&value) {
            IC_LIGHT_MODE
        } else {
            IC_DARK_MODE
        };
        vars.set("dark_icon", icon.to_string());
    }
}

impl Default for Darkmode {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_dark_true_when_prefer_dark() {
        assert!(is_dark("'prefer-dark'\n"));
    }

    #[test]
    fn is_dark_false_when_prefer_light() {
        assert!(!is_dark("'prefer-light'\n"));
    }

    #[test]
    fn is_dark_true_when_unset_or_gsettings_fails() {
        // darkmode.sh:10 — an unset/failed gsettings read counts as dark.
        assert!(is_dark(""));
        assert!(is_dark("'default'"));
    }
}
