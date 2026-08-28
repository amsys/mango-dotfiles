//! Now-playing pill (T28) — replaces ironbar's native `music` module.
//! Confirmed against the vendored ironbar source (v0.19.0,
//! `modules/music/mod.rs`): that module renders through
//! `label.set_label_escaped(...)`, so it can never carry the two-line Pango
//! markup `mango.rs::window_text` uses for the window-title pill. A
//! `custom` module fed by this collector's own markup can, so the two
//! pills read as one visual language (dim app/artist line above, plain
//! title below) instead of one bold `title / artist` line.
//!
//! One long-lived `playerctl --follow` child ([`crate::net::MonitorChild`])
//! supplies both the initial state and every change, no polling: `--follow`
//! prints the templated fields once on start, then again only when one of
//! them changes (confirmed live — a template with no `position` token never
//! re-fires on playback progress, only on a status or track change).

use crate::mango::truncate_ellipsis;
use crate::net::MonitorChild;
use crate::tooltip::{dim, esc, set_titled};
use crate::vars::Vars;

/// ASCII unit separator — never appears in real MPRIS metadata, so a naive
/// split is safe (same reasoning docker.rs gives for its `|`-delimited
/// `FORMAT`, one level more paranoid since a track title can contain `|`).
const SEP: char = '\u{1f}';
const FORMAT: &str = "{{status}}\u{1f}{{artist}}\u{1f}{{title}}\u{1f}{{playerName}}";

/// Trimmed harder than `win`'s 45 chars (mango.rs) — `music` sits mid-block
/// in `audio_modules()`, not at a growth end (statusbar-layout.md INV-1),
/// so its max width has to stay small on purpose, not just capped.
const TITLE_MAX: usize = 24;

struct Parsed {
    status: String,
    artist: String,
    title: String,
    player: String,
}

pub struct Music {
    pub mon: MonitorChild,
    last_line: Option<String>,
    state: Option<Parsed>,
}

impl Music {
    pub fn new() -> Self {
        Self {
            // ponytail: a player quitting mid-EOF leaves the last-known
            // track on the bar until playerctl reconnects (MonitorChild's
            // restart loop blocks inside next_line() with no "just
            // respawned" signal this collector could clear on) — accepted
            // since this machine's kdeconnect MPRIS bridge is always
            // present (confirmed live: `playerctl -l` lists it even with
            // nothing playing), so playerctl never actually runs out of
            // players to follow. Upgrade: a max-silence timeout in
            // MonitorChild if a box without that bridge ever needs this.
            mon: MonitorChild::new("playerctl", &["--follow", "--format", FORMAT, "status"]),
            last_line: None,
            state: None,
        }
    }

    /// Raw-line dedup only, same shape as docker.rs's `ingest_line` — a
    /// `--follow` field that hasn't actually changed never reaches here in
    /// the first place, but the exact same guard costs nothing to keep.
    pub fn ingest_line(&mut self, line: &str) -> bool {
        if self.last_line.as_deref() == Some(line) {
            return false;
        }
        self.last_line = Some(line.to_string());
        let mut f = line.splitn(4, SEP);
        self.state = Some(Parsed {
            status: f.next().unwrap_or("").to_string(),
            artist: f.next().unwrap_or("").to_string(),
            title: f.next().unwrap_or("").to_string(),
            player: f.next().unwrap_or("").to_string(),
        });
        true
    }

    /// Pure render off whatever `ingest_line` last parsed — no fork, so
    /// safe to call every time a line arrives.
    pub fn apply(&self, vars: &mut Vars) {
        let Some(s) = &self.state else { return };
        // Nothing loaded (kdeconnect's always-present bridge with no
        // track, or any player between tracks) — hide, don't show a bare
        // player name. A paused-but-loaded track still shows: pausing
        // isn't "nothing playing".
        if s.title.is_empty() {
            vars.set("music_on", "false");
            vars.set("music_text", "");
            set_titled(vars, "music_tip", "Now Playing", String::new());
            return;
        }
        let head = if s.artist.is_empty() { &s.player } else { &s.artist };
        let shown = truncate_ellipsis(&s.title, TITLE_MAX);
        let text = format!(
            "<span size=\"small\" alpha=\"70%\">{}</span>\n{}",
            esc(head),
            esc(&shown)
        );
        let body = format!(
            "{}\n{}",
            esc(&s.title),
            dim(&format!("{} \u{b7} {}", esc(head), esc(&s.status)))
        );

        vars.set("music_on", "true");
        vars.set("music_text", text);
        set_titled(vars, "music_tip", "Now Playing", body);
    }
}

impl Default for Music {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_line_dedups_identical_lines() {
        let mut m = Music::new();
        assert!(m.ingest_line("Playing\u{1f}Boards of Canada\u{1f}Roygbiv\u{1f}spotify"));
        assert!(!m.ingest_line("Playing\u{1f}Boards of Canada\u{1f}Roygbiv\u{1f}spotify"));
        assert!(m.ingest_line("Paused\u{1f}Boards of Canada\u{1f}Roygbiv\u{1f}spotify"));
    }

    #[test]
    fn apply_hides_when_title_is_empty() {
        let mut m = Music::new();
        m.ingest_line("Paused\u{1f}\u{1f}\u{1f}kdeconnect");
        let mut vars = Vars::new();
        m.apply(&mut vars);
        while let Some((k, _)) = vars.peek_dirty().map(|(k, v)| (k.to_string(), v.to_string())) {
            vars.ack(&k);
        }
        assert_eq!(vars.live_value("music_on"), Some("false"));
        assert_eq!(vars.live_value("music_text"), Some(""));
    }

    #[test]
    fn apply_falls_back_to_player_name_with_no_artist() {
        let mut m = Music::new();
        m.ingest_line("Playing\u{1f}\u{1f}Some Track\u{1f}mpv");
        let mut vars = Vars::new();
        m.apply(&mut vars);
        let mut text = None;
        while let Some((k, v)) = vars.peek_dirty().map(|(k, v)| (k.to_string(), v.to_string())) {
            if k == "music_text" {
                text = Some(v.clone());
            }
            vars.ack(&k);
        }
        let text = text.expect("music_text must be set");
        assert!(text.contains("mpv"), "expected player-name fallback: {text}");
        assert!(text.contains("Some Track"));
    }

    #[test]
    fn apply_truncates_a_long_title() {
        let mut m = Music::new();
        let long = "a".repeat(60);
        m.ingest_line(&format!("Playing\u{1f}Artist\u{1f}{long}\u{1f}spotify"));
        let mut vars = Vars::new();
        m.apply(&mut vars);
        while let Some((k, v)) = vars.peek_dirty().map(|(k, v)| (k.to_string(), v.to_string())) {
            if k == "music_text" {
                assert!(v.contains('\u{2026}'), "expected an ellipsis in: {v}");
                assert!(!v.contains(&long), "title must be cut, not shown whole");
            }
            vars.ack(&k);
        }
    }
}
