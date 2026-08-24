//! Keep-awake pill: one systemd user unit, `mango-keepawake.service`,
//! holding a `systemd-inhibit --what=idle:handle-lid-switch --mode=block`
//! inhibitor while active. `sleep` is deliberately not in `--what` — an
//! explicit `systemctl suspend` (SUPER+SHIFT+L, battery-guard.sh's 5%
//! emergency suspend) must still work while this is on.
//!
//! Replaces ironbar's native `inhibit` module. That module's
//! `gtk_application_inhibit()` call needs the `org.freedesktop.portal.
//! Inhibit` D-Bus interface; this session's xdg-desktop-portal falls back
//! to its `default=gtk` backend (`XDG_CURRENT_DESKTOP=mango`, no
//! wlroots-specific preference), and xdg-desktop-portal-gtk implements
//! Inhibit by proxying `org.gnome.SessionManager` — not present on this
//! bus. Confirmed live, three times, in
//! `~/.local/share/ironbar/ironbar.*.log`:
//! `Cannot get portal org.freedesktop.portal.Inhibit version:
//! ... No such interface`. No config change fixes that; the toggle was
//! changing a glyph and inhibiting nothing. `systemd-inhibit` talks to
//! logind directly and needs no portal.
//!
//! Same shape as remote.rs: one unit, `is_active()` poll, no event stream
//! (keepawake.sh's `--toggle` pokes `mango-bard refresh keepawake` on the
//! actual state-change edge, and main.rs gives this an explicit startup
//! refresh) — but one unit instead of two, and the glyph swaps with state
//! (darkmode.rs's shape) rather than staying fixed, carried over from the
//! native module's own `format_on`/`format_off` pair so the pill still
//! reads as a state at a glance, not just an availability flag.

use crate::mango::CLASS_PREFIX;
use crate::tooltip::{kv, sect, set_titled};
use crate::vars::Vars;
use tokio::process::Command;

/// nf-md-coffee, filled cup on a saucer — shown while active. Carried over
/// from the old `inhibit_module()`'s `format_on` (genconfig.rs); see that
/// module's own T15/T17/T18 history for why this exact codepoint.
const IC_ON: char = '\u{f0176}';
/// nf-md-coffee_outline — shown while inactive. Same 832/1000 em advance as
/// IC_ON in the Propo font (style.css), so the pill doesn't change width on
/// toggle.
const IC_OFF: char = '\u{f06ca}';

const UNIT: &str = "mango-keepawake.service";

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

/// `systemctl --user is-active <unit>` as a plain bool — matches the exit
/// code, not the printed status text (remote.rs's own `is_active`, ported
/// verbatim: a masked/failed unit reads as down the same as a stopped one).
async fn is_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", unit])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub struct Keepawake {
    active: bool,
}

impl Keepawake {
    pub fn new() -> Self {
        Self { active: false }
    }

    pub async fn refresh(&mut self, vars: &mut Vars) {
        self.active = is_active(UNIT).await;

        vars.set(
            "inhibit_text",
            (if self.active { IC_ON } else { IC_OFF }).to_string(),
        );

        let mut tip = String::new();
        tip.push_str(&sect("", "blocks idle lock/dpms/suspend + lid close"));
        tip.push_str(&kv("State", if self.active { "on" } else { "off" }));
        set_titled(
            vars,
            "inhibit_tip",
            "Keep-awake",
            tip.trim_end_matches('\n').to_string(),
        );

        vars.set(&class_key("inhibit"), if self.active { "active" } else { "" });
    }
}

impl Default for Keepawake {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_inactive() {
        let k = Keepawake::new();
        assert!(!k.active);
    }
}
