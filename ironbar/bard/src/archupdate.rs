//! Arch-Update pending-count pill (T28) — promoted out of the tray drawer:
//! "packages are waiting" is glance-worthy, unlike the tray icon it replaces
//! on the bar (Nextcloud/ZapZap/NordVPN stay tray-only).
//!
//! No fork, ever. `arch-update.timer`'s own `arch-update --check` (the
//! package's own systemd user unit — T32 dropped the redundant tray applet
//! from config.conf's exec-once list, this bar pill replaces it) writes its
//! whole state to plain files under `$XDG_STATE_HOME/arch-update/` on every
//! check — `tray_icon` in place (`echo ... > tray_icon`, lib/common.sh:
//! 209/215, so `IN_CLOSE_WRITE` on that one inode is the correct watch,
//! same reasoning as powermode.rs's own doc comment) — so watching that one
//! file and re-reading its two count files alongside it is the whole
//! collector. `lib/tray.sh` (the tray's own bootstrap) only `touch`es these
//! files if missing; it never writes real content, so it was never this
//! collector's actual data source even when the tray was still running.

use crate::mango::CLASS_PREFIX;
use crate::sys::FileWatch;
use crate::tooltip::{barico_label, esc, set_titled};
use crate::vars::Vars;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;

/// md-package_up — render-checked with pango-view (repo convention) against
/// nerd-fonts' own glyphnames.json: a box with an up-arrow reads as
/// "update available", not just "package".
const IC_UPDATE: char = '\u{f03d5}';

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

fn state_dir() -> PathBuf {
    std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("arch-update")
}

fn count_lines(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
}

pub struct Archupdate {
    watch: FileWatch,
    dir: PathBuf,
}

impl AsRawFd for Archupdate {
    fn as_raw_fd(&self) -> RawFd {
        self.watch.as_raw_fd()
    }
}

impl Archupdate {
    pub fn new() -> std::io::Result<Self> {
        let dir = state_dir();
        let watch = FileWatch::new(dir.join("tray_icon").to_str().expect("path is valid UTF-8"))?;
        Ok(Self { watch, dir })
    }

    /// Call after the fd is readable — mirrors `PowermodeWatch::on_event`.
    pub fn on_event(&mut self) {
        self.watch.drain();
    }

    /// Pure file reads, no fork — safe to call on every event and at
    /// startup alike (T6a's `refresh()` convention: pure state reads never
    /// need a `_due` debounce).
    pub fn refresh(&self, vars: &mut Vars) {
        let total = count_lines(&self.dir.join("last_updates_check_packages"))
            + count_lines(&self.dir.join("last_updates_check_aur"));
        if total == 0 {
            vars.set("au_show", "false");
            vars.set("au_text", "");
            set_titled(vars, "au_tip", "Arch-Update", String::new());
            // T29: `archupdate` shares `devload`'s node with `claude`/
            // `docker` now (`devload_module()`'s own doc comment) — ""
            // clears this slot only, so it stays bare, not prefixed.
            vars.set(&class_key("devload#au"), "");
            return;
        }
        let list = std::fs::read_to_string(self.dir.join("last_updates_check")).unwrap_or_default();
        let body = list
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(esc)
            .collect::<Vec<_>>()
            .join("\n");

        vars.set("au_show", "true");
        vars.set("au_text", format!("{} {total}", barico_label(IC_UPDATE)));
        set_titled(vars, "au_tip", "Arch-Update", body);
        // "pending", not "warning" — updates waiting is routine, not an
        // error state; style.css maps this to @accent, not @urgent.
        // T29: `au-` prefix keeps this slot's value out of `claude`'s/
        // `docker`'s own value space on the shared `devload` node.
        vars.set(&class_key("devload#au"), "au-pending");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_lines_ignores_blank_lines() {
        let tmp = std::env::temp_dir().join(format!("archupdate-test-{}", std::process::id()));
        std::fs::write(&tmp, "a\nb\n\nc\n").unwrap();
        assert_eq!(count_lines(&tmp), 3);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn count_lines_is_zero_for_a_missing_file() {
        assert_eq!(
            count_lines(std::path::Path::new("/nonexistent/does-not-exist")),
            0
        );
    }
}
