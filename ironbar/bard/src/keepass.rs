//! KeePassXC lock-state pill (T28) — promoted out of the tray drawer since
//! lock state is glance-worthy, not just click-worthy, unlike the other four
//! tray items (Nextcloud, ZapZap, NordVPN, Arch-Update's icon itself).
//!
//! Reads the same freedesktop Secret Service property
//! `wait-for-keepass-unlock.sh` already parses (`org.freedesktop.Secret.
//! Collection`'s `Locked`), resolved the same way that script does — via
//! the `default` alias, never a hardcoded collection name (KeePassXC names
//! its collection per-database; only `ReadAlias s default` is portable
//! across machines).

use crate::mango::CLASS_PREFIX;
use crate::net::MonitorChild;
use crate::tooltip::{barico_label, set_titled};
use crate::vars::Vars;
use tokio::process::Command;
use tokio::time::timeout;

use crate::cmd::CMD_TIMEOUT;

/// T29 follow-up: md-key_outline / md-key — swapped from the original
/// md-lock/md-lock_open pair (user preference: a key reads more directly
/// as "this needs a key to open" than the padlock's own two states, which
/// look too similar at 15px to tell apart at a glance). Hollow for locked
/// (nothing to see, safe resting state — same "hollow means nothing to
/// notice" convention `.keepass.unlocked`'s own doc comment already
/// applies), solid for unlocked (a real key exists, worth a glance).
/// Render-checked with `pango-view -q --font="JetBrainsMono Nerd Font
/// Propo 40"` before wiring in (repo convention — T8b/T15/T17/T18's
/// repeated wrong-glyph bugs): both are unambiguous keys, not the
/// thumbtack U+F0403 turned out to be during the original lock-icon check.
const IC_LOCKED: char = '\u{f0dd6}'; // md-key_outline
const IC_UNLOCKED: char = '\u{f0306}'; // md-key

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

pub struct Keepass {
    pub mon: MonitorChild,
}

impl Keepass {
    pub fn new() -> Self {
        Self {
            // ponytail: unscoped to the whole `org.freedesktop.secrets`
            // service, not just our collection's object path — busctl has
            // no member/path filter, and resolving the path needs an async
            // fork this sync constructor can't make. Every Secret Service
            // call from any app (browser extensions included) triggers a
            // refresh; each refresh is two cheap forks (ReadAlias +
            // get-property), so this is wasted work, not wrong output.
            // Upgrade: scope with `gdbus monitor --object-path <resolved>`
            // once the unscoped noise is measurably a problem.
            mon: MonitorChild::new(
                "busctl",
                &["--user", "monitor", "--json=short", "org.freedesktop.secrets"],
            ),
        }
    }

    /// Mirrors wait-for-keepass-unlock.sh's `is_unlocked()` exactly: resolve
    /// the `default` alias to a collection path, then read that path's
    /// `Locked` property. Either fork failing (no KeePassXC running, no
    /// database open yet) hides the pill rather than showing stale state.
    pub async fn refresh(&mut self, vars: &mut Vars) {
        let Some(path) = self.resolve_path().await else {
            self.hide(vars);
            return;
        };
        let out = timeout(
            CMD_TIMEOUT,
            Command::new("busctl")
                .args([
                    "--user",
                    "get-property",
                    "org.freedesktop.secrets",
                    &path,
                    "org.freedesktop.Secret.Collection",
                    "Locked",
                ])
                .output(),
        )
        .await;
        let locked = match out {
            Ok(Ok(o)) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim() == "b true"
            }
            _ => {
                self.hide(vars);
                return;
            }
        };

        let icon = if locked { IC_LOCKED } else { IC_UNLOCKED };
        vars.set("kp_text", barico_label(icon));
        set_titled(
            vars,
            "kp_tip",
            "KeePassXC",
            if locked { "Locked" } else { "Unlocked" }.to_string(),
        );
        // Unlocked gets the visible class, not locked — locked is the safe
        // resting state (nothing to notice); an open database is the one
        // worth a glance.
        vars.set(&class_key("keepass"), if locked { "" } else { "unlocked" });
    }

    async fn resolve_path(&self) -> Option<String> {
        let out = timeout(
            CMD_TIMEOUT,
            Command::new("busctl")
                .args([
                    "--user",
                    "call",
                    "org.freedesktop.secrets",
                    "/org/freedesktop/secrets",
                    "org.freedesktop.Secret.Service",
                    "ReadAlias",
                    "s",
                    "default",
                ])
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        if !out.status.success() {
            return None;
        }
        // busctl prints `o "/path"` — same shape parse_path() (awk -F'"')
        // in wait-for-keepass-unlock.sh reads.
        let raw = String::from_utf8_lossy(&out.stdout);
        let path = raw.split('"').nth(1)?.to_string();
        (!path.is_empty()).then_some(path)
    }

    fn hide(&self, vars: &mut Vars) {
        vars.set("kp_text", "");
        set_titled(vars, "kp_tip", "KeePassXC", String::new());
        vars.set(&class_key("keepass"), "");
    }
}

impl Default for Keepass {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_key_uses_the_shared_class_prefix() {
        assert_eq!(class_key("keepass"), "@class/keepass");
    }
}
