//! Remote-access pill: wayvnc (screen) + kdeconnectd (input/clipboard/
//! files), one pill, two switches — left click toggles VNC (and starts
//! kdeconnectd with it), right click toggles kdeconnectd alone. Mirrors
//! hotspot.rs's shape but
//! stays visible while down — see genconfig.rs's `remote_module` doc
//! comment for why `show_if` isn't reused here (a toggle that hides itself
//! when off has no way back on).
//!
//! No event stream to watch (same reasoning as hotspot.rs/darkmode.rs):
//! `remote.sh --toggle-vnc`/`--toggle-kdeconnect` pokes `mango-bard refresh
//! remote` on the actual state-change edge, and main.rs gives this an
//! explicit startup refresh.
//!
//! Candidate glyph, not live-verified: 0xF0379 (nf-md-monitor). This
//! codebase has repeatedly found Material-Symbols-family codepoints render
//! as tofu under GTK4 here (hotspot.rs's IC_HOTSPOT history) — check a
//! `grim -o <output>` crop of the bar after building and swap the codepoint
//! if it doesn't render.

use crate::cmd::is_active;
use crate::mango::CLASS_PREFIX;
use crate::tooltip::{barico, dim, dot, good, kv, set_titled};
use crate::vars::Vars;

const IC_REMOTE: char = '\u{f0379}';

const VNC_UNIT: &str = "wayvnc.service";
const KDECONNECT_UNIT: &str = "kdeconnectd.service";

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

fn runtime_dir() -> std::path::PathBuf {
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(runtime).join("mango-remote")
}

/// Path `remote.sh --privacy-watch` drops while it has blanked every output
/// for a connected VNC client (see that script's own header). Existence
/// alone is the signal — no need to parse its contents here.
fn privacy_state_path() -> std::path::PathBuf {
    runtime_dir().join("wlopm-state")
}

/// True while a VNC client is connected and the local panels are blanked.
/// `pomo.rs` uses it to refuse the idle-unlock autostart: remote input resets
/// hypridle the same as local input, so without this a remote session starts
/// a work block nobody asked for.
pub(crate) fn vnc_client_connected() -> bool {
    privacy_state_path().exists()
}

/// `<monitor> tag <n>`, read from `remote.sh`'s own `pulled-origin` file (its
/// header names the format: `"<monitor>\t<tag>\n"`), or `None` if nothing is
/// pulled. wayvnc is pinned to the virtual output now (`--vnc-exec`), so the
/// old `Screen` row this replaced — which output wayvnc *captures* — always
/// read `HEADLESS-<n>` and told the user nothing; which physical tag is
/// pulled onto it is the fact worth showing.
fn pulled_line() -> Option<String> {
    let raw = std::fs::read_to_string(runtime_dir().join("pulled-origin")).ok()?;
    let (mon, tag) = raw.trim().split_once('\t')?;
    Some(format!("{mon} tag {tag}"))
}

/// On/off cell for a service row: a coloured dot plus the word, the same
/// dot-then-text shape docker.rs uses for container state. A dot alone is
/// ambiguous at a glance; the word alone carries no colour. "off" stays
/// uncoloured on purpose — remote access down is the safe resting state,
/// not a fault, so `bad()` red would read as an alarm for normal use.
fn state(up: bool) -> String {
    if up {
        format!("{} {}", dot("good"), good("on"))
    } else {
        format!("{} off", dot("dim"))
    }
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
        tip.push_str(&kv("VNC", &state(self.vnc_up)));
        if self.vnc_up {
            // Own row, not a "(dark)" suffix on Pulled: blanked outputs are
            // a separate fact from which tag is pulled, and the reader needs
            // to see at a glance that the local screens are off.
            tip.push_str(&kv(
                "Screens",
                &if privacy_state_path().exists() {
                    format!("{} blanked", dot("warn"))
                } else {
                    format!("{} live", dot("good"))
                },
            ));
            tip.push_str(&kv(
                "Pulled",
                &pulled_line().unwrap_or_else(|| "nothing".to_string()),
            ));
        }
        tip.push_str(&kv("KDE Conn", &state(self.kdeconnect_up)));
        tip.push('\n');
        tip.push_str(&dim("VPN tunnel + hotspot only"));
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

    #[test]
    fn state_colours_on_but_leaves_off_neutral() {
        use crate::tooltip::{C_DIM, C_GOOD};
        let on = state(true);
        assert!(on.contains("on"));
        assert!(on.contains(&C_GOOD.to_string()), "on is green: {on}");
        let off = state(false);
        assert!(off.contains("off"));
        assert!(off.contains(&C_DIM.to_string()), "off dot is dim: {off}");
        assert!(
            !off.contains(&crate::tooltip::C_BAD.to_string()),
            "off is the safe resting state, never red: {off}"
        );
    }
}
