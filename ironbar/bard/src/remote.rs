//! Remote-access pill: wayvnc (screen) + kdeconnectd (input/clipboard/
//! files), one button, one on/off state. Mirrors hotspot.rs's shape but
//! stays visible while down — see genconfig.rs's `remote_module` doc
//! comment for why `show_if` isn't reused here (a toggle that hides itself
//! when off has no way back on).
//!
//! No event stream to watch (same reasoning as hotspot.rs/darkmode.rs):
//! `remote.sh --toggle` pokes `mango-bard refresh remote` on the actual
//! state-change edge, and main.rs gives this an explicit startup refresh.
//!
//! Candidate glyph, not live-verified: 0xF0379 (nf-md-monitor). This
//! codebase has repeatedly found Material-Symbols-family codepoints render
//! as tofu under GTK4 here (hotspot.rs's IC_HOTSPOT history) — check a
//! `grim -o <output>` crop of the bar after building and swap the codepoint
//! if it doesn't render.

use crate::cmd::is_active;
use crate::mango::CLASS_PREFIX;
use crate::tooltip::{barico, kv, sect, set_titled};
use crate::vars::Vars;

const IC_REMOTE: char = '\u{f0379}';

const VNC_UNIT: &str = "wayvnc.service";
const KDECONNECT_UNIT: &str = "kdeconnectd.service";

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

pub struct Remote {
    vnc_up: bool,
    kdeconnect_up: bool,
}

impl Remote {
    pub fn new() -> Self {
        Self {
            vnc_up: false,
            kdeconnect_up: false,
        }
    }

    pub async fn refresh(&mut self, vars: &mut Vars) {
        self.vnc_up = is_active(VNC_UNIT).await;
        self.kdeconnect_up = is_active(KDECONNECT_UNIT).await;

        vars.set("remote_text", barico(IC_REMOTE));

        let mut tip = String::new();
        tip.push_str(&sect("", "wg_hetzner + hotspot only"));
        tip.push_str(&kv("VNC", if self.vnc_up { "on" } else { "off" }));
        tip.push_str(&kv(
            "KDE Connect",
            if self.kdeconnect_up { "on" } else { "off" },
        ));
        set_titled(
            vars,
            "remote_tip",
            "Remote access",
            tip.trim_end_matches('\n').to_string(),
        );

        let class = match (self.vnc_up, self.kdeconnect_up) {
            (true, true) => "active",
            (true, false) | (false, true) => "partial",
            (false, false) => "",
        };
        vars.set(&class_key("remote"), class);
    }
}

impl Default for Remote {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_both_down() {
        let r = Remote::new();
        assert!(!r.vnc_up);
        assert!(!r.kdeconnect_up);
    }
}
