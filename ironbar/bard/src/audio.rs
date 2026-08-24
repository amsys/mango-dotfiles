//! The audio collector: volume/mic pills plus the sink/mic/per-app popup.
//! Ports src/waybar/scripts/volume.sh (`custom/volume`) and
//! `pulseaudio#source`'s mute glyph (config.jsonc:314-318) — see IRONBAR.md
//! T4 for the design. `custom/volwatch`'s `pactl subscribe` coproc is
//! replaced by [`crate::net::MonitorChild`] (already respawn/backoff-shaped
//! from T3) so nothing new needs inventing for the child-process lifecycle.
//!
//! `refresh()` is the only thing that forks — five processes, the same set
//! `volume.sh` makes per run: `pactl get-default-sink`/`get-default-source`
//! and `pactl -f json list sinks`/`sources`/`sink-inputs`. `jq` disappears
//! entirely; `serde_json` (already a dependency) does the parsing.

use crate::cmd::run;
use crate::mango::CLASS_PREFIX;
use crate::net::MonitorChild;
use crate::tooltip::{bad, bar, barico, dim, esc, good, grade, row, sect, set_titled, C_EMPTY, C_GOOD};
use crate::vars::Vars;
use serde_json::Value;

// ------------------------------------------------------------------ icons
//
// Codepoints copied verbatim from volume.sh's own printf literals (its
// inline comments name the Nerd Font glyph but are not always the exact
// codepoint printed — the literal byte sequence is the source of truth).

const IC_OUT: char = '\u{f04c3}'; // md-speaker
const IC_MIC: char = '\u{f036c}'; // md-microphone
const IC_APP: char = '\u{f075a}'; // md-music-note
/// config.jsonc:317 `format-source-muted` — waybar's own built-in
/// `pulseaudio#source` module, unrelated to volume.sh, printed unwrapped
/// (no `barico()`), matching that module's plain-glyph format string.
/// T8b: U+E02B (waybar's own Material Symbols "mic_off") -> U+F131
/// (mic-off, JetBrainsMono Nerd Font Font Awesome) — GTK4 cannot correctly
/// rasterize Material Symbols Rounded's variable font on this system; see
/// IRONBAR.md's T8b entry. Unlike IC_OUT/IC_MIC/IC_APP/ic_vol() above
/// (all mdi-range Nerd Font glyphs baked into the static JBNF font file,
/// confirmed rendering correctly), this one specifically needed a swap.
/// T19: U+F131 -> U+F036D (md-microphone_off) — one-icon-family sweep
/// (IRONBAR.md T19), same reasoning as genconfig.rs's colorpicker/snip swap.
const IC_MIC_MUTED: char = '\u{f036d}';

/// volume.sh:27-34 — the bar glyph, matching the ramp waybar's
/// `format-icons` used. 0 and `<34` share a glyph in the original, so the
/// port collapses the redundant `-eq 0` branch into the `<34` arm.
pub fn ic_vol(pct: i64, muted: bool) -> char {
    if muted {
        '\u{f075f}'
    } else if pct < 34 {
        '\u{f057f}'
    } else if pct < 67 {
        '\u{f0580}'
    } else {
        '\u{f057e}'
    }
}

/// T9: opacity-by-level class (`.volume.quiet/mid/loud`, `style.css`),
/// requested alongside dropping the bar text's `NN%` suffix so the icon
/// alone still hints at the level. `muted` gets no separate bucket — the
/// glyph itself already swaps (`ic_vol()` above), so no opacity trick is
/// needed on top of that.
pub fn volume_class(pct: i64, muted: bool) -> &'static str {
    if muted {
        "muted"
    } else if pct < 34 {
        "quiet"
    } else if pct < 67 {
        "mid"
    } else {
        "loud"
    }
}

// ---------------------------------------------------------------- parsing

#[derive(Debug, Clone, PartialEq)]
pub struct Dev {
    pub pct: i64,
    pub muted: bool,
    pub label: String,
    pub port: String,
    pub profile: String,
}

/// volume.sh:41-50 `dev_line()`: find the sink/source whose `name` matches
/// `default_name` and pull volume/mute/label/port/profile out of it. `jq`'s
/// `//` only falls through on a missing (`null`) key, never on an empty
/// string, so the label fallback chain below does the same.
fn dev_line(list: &Value, default_name: &str) -> Option<Dev> {
    if default_name.is_empty() {
        return None;
    }
    let dev = list
        .as_array()?
        .iter()
        .find(|d| d.get("name").and_then(Value::as_str) == Some(default_name))?;

    let pct = dev
        .get("volume")?
        .as_object()?
        .values()
        .next()?
        .get("value_percent")?
        .as_str()?
        .trim_end_matches('%')
        .parse()
        .ok()?;
    let muted = dev.get("mute").and_then(Value::as_bool).unwrap_or(false);
    let props = dev.get("properties");
    let label = props
        .and_then(|p| p.get("node.nick"))
        .and_then(Value::as_str)
        .or_else(|| dev.get("description").and_then(Value::as_str))
        .or_else(|| dev.get("name").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let port = dev
        .get("active_port")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_start_matches("[In] ")
        .trim_start_matches("[Out] ")
        .to_string();
    let profile = props
        .and_then(|p| p.get("device.profile.description"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    Some(Dev {
        pct,
        muted,
        label,
        port,
        profile,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub pct: i64,
    pub muted: bool,
    pub name: String,
    pub title: String,
}

/// volume.sh:54-63 `app_lines()`: streams currently playing, loudest first,
/// corked (paused) streams dropped.
fn app_lines(list: &Value) -> Vec<App> {
    let Some(arr) = list.as_array() else {
        return Vec::new();
    };
    let mut apps: Vec<App> = arr
        .iter()
        .filter(|s| !s.get("corked").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|s| {
            let pct = s
                .get("volume")?
                .as_object()?
                .values()
                .next()?
                .get("value_percent")?
                .as_str()?
                .trim_end_matches('%')
                .parse()
                .ok()?;
            let muted = s.get("mute").and_then(Value::as_bool).unwrap_or(false);
            let props = s.get("properties");
            let name = props
                .and_then(|p| p.get("application.name"))
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let title = props
                .and_then(|p| p.get("media.name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(App {
                pct,
                muted,
                name,
                title,
            })
        })
        .collect();
    apps.sort_by_key(|a| std::cmp::Reverse(a.pct));
    apps
}

/// volume.sh:128-129's awk: drop whichever of `port`/`profile` repeats
/// `label` or each other (this machine's HiFi card reports the nick, the
/// port and the profile description as all literally "Speaker" — printing
/// all three reads as a bug), join survivors with "  ·  ".
fn detail(label: &str, port: &str, profile: &str) -> Option<String> {
    let mut seen: Vec<&str> = Vec::new();
    for v in [port, profile] {
        if v.is_empty() || v == label || seen.contains(&v) {
            continue;
        }
        seen.push(v);
    }
    if seen.is_empty() {
        None
    } else {
        Some(seen.join("  ·  "))
    }
}

/// volume.sh:73 — pactl reports client connect/disconnect too (every
/// `wpctl set-volume` invocation is one), and refreshing on those would be
/// pure noise: only sink/source/server transitions are regrade-worthy.
pub fn event_matches(line: &str) -> bool {
    line.contains(" on sink") || line.contains(" on source") || line.contains(" on server")
}

// ------------------------------------------------------------------ popup

/// volume.sh:117-154's `TIP=$(...)` build, ported section by section.
/// `row()`/`dim()` carry no trailing newline of their own (tooltip.rs), so
/// each call here is followed by an explicit `\n` — same shape
/// `window_list()` already uses, and matching how bash's `$(...)` strips
/// every trailing newline from the final result regardless.
fn build_tip(dev: &Dev, mic: Option<&Dev>, apps: &[App]) -> String {
    let mut tip = String::new();
    tip.push_str(&sect(&IC_OUT.to_string(), "Output"));
    tip.push_str(&row(&format!(
        "{:>3}%  {}",
        dev.pct,
        bar(dev.pct, grade(dev.pct, 101, 130), 20)
    )));
    tip.push('\n');
    let out_muted = if dev.muted {
        format!("  ·  {}", bad("muted"))
    } else {
        String::new()
    };
    tip.push_str(&row(&format!("{}{}", esc(&dev.label), out_muted)));
    tip.push('\n');
    if let Some(d) = detail(&dev.label, &dev.port, &dev.profile) {
        tip.push_str(&dim(&esc(&d)));
        tip.push('\n');
    }

    if let Some(mic) = mic {
        tip.push_str(&sect(&IC_MIC.to_string(), "Microphone"));
        let color = if mic.muted { C_EMPTY } else { C_GOOD };
        tip.push_str(&row(&format!(
            "{:>3}%  {}",
            mic.pct,
            bar(mic.pct, color, 20)
        )));
        tip.push('\n');
        let mic_muted = if mic.muted {
            format!("  ·  {}", good("muted"))
        } else {
            String::new()
        };
        tip.push_str(&row(&format!("{}{}", esc(&mic.label), mic_muted)));
        tip.push('\n');
    }

    tip.push_str(&sect(&IC_APP.to_string(), "Playing"));
    if apps.is_empty() {
        tip.push_str(&dim("nothing"));
        tip.push('\n');
    } else {
        for app in apps.iter().take(5) {
            let color = if app.muted { C_EMPTY } else { C_GOOD };
            tip.push_str(&row(&format!(
                "{} {:>4}%  {}",
                bar(app.pct, color, 10),
                app.pct,
                esc(&app.name)
            )));
            tip.push('\n');
            if !app.title.is_empty() && app.title != app.name {
                let truncated: String = app.title.chars().take(44).collect();
                tip.push_str(&dim(&esc(&truncated)));
                tip.push('\n');
            }
        }
    }

    tip.trim_end_matches('\n').to_string()
}

// ------------------------------------------------------------------ Audio

pub struct Audio {
    pub pactl_mon: MonitorChild,
    last_line: Option<String>,
}

impl Audio {
    pub fn new() -> Self {
        Self {
            pactl_mon: MonitorChild::new("pactl", &["subscribe"]),
            last_line: None,
        }
    }

    /// Raw-line dedup plus `event_matches()` filtering. Returns whether this
    /// line is regrade-worthy — unlike net.rs's nmcli ingest (where every
    /// new line matters), most `pactl subscribe` lines are client noise, so
    /// only sink/source/server transitions return true.
    pub fn ingest_line(&mut self, line: &str) -> bool {
        if self.last_line.as_deref() == Some(line) {
            return false;
        }
        self.last_line = Some(line.to_string());
        event_matches(line)
    }

    /// Re-derives `vol_text`/`vol_tip`/`mic_text` from a fresh fork-and-parse
    /// pass. The only thing in audio.rs that forks.
    pub async fn refresh(&mut self, vars: &mut Vars) {
        let sink_name = first_line(&run("pactl", &["get-default-sink"]).await);
        let sinks = parse_json(&run("pactl", &["-f", "json", "list", "sinks"]).await);
        let dev = dev_line(&sinks, &sink_name);

        let source_name = first_line(&run("pactl", &["get-default-source"]).await);
        let sources = parse_json(&run("pactl", &["-f", "json", "list", "sources"]).await);
        let mic = dev_line(&sources, &source_name);

        // pulseaudio#source (config.jsonc:314-318): independent of whether a
        // sink exists at all.
        let mic_text = match &mic {
            Some(m) if m.muted => IC_MIC_MUTED.to_string(),
            _ => String::new(),
        };
        vars.set("mic_text", mic_text);

        match &dev {
            None => {
                // volume.sh:102-105 — no sink at all: plain text, no markup.
                vars.set("vol_text", "");
                set_titled(vars, "vol_tip", "Volume", "No audio sink".to_string());
            }
            Some(d) => {
                let sink_inputs =
                    parse_json(&run("pactl", &["-f", "json", "list", "sink-inputs"]).await);
                let apps = app_lines(&sink_inputs);
                // T9: icon only — the percentage moved into the popup this
                // pill already opens on click, to stop crowding the
                // wifi/eth/netsec trio next to it.
                vars.set("vol_text", barico(ic_vol(d.pct, d.muted)));
                vars.set(
                    &format!("{CLASS_PREFIX}volume"),
                    volume_class(d.pct, d.muted),
                );
                set_titled(vars, "vol_tip", "Volume", build_tip(d, mic.as_ref(), &apps));
            }
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- helpers

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn parse_json(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or(Value::Array(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures trimmed to the fields dev_line()/app_lines() actually read,
    // captured against a real `pactl -f json list sinks` on this machine
    // (IRONBAR.md T4 verification).
    const SINKS_FIXTURE: &str = r#"[
        {
            "name": "alsa_output.speaker",
            "mute": false,
            "volume": { "front-left": { "value_percent": "40%" } },
            "properties": { "node.nick": "Speaker", "device.profile.description": "Speaker" },
            "active_port": "[Out] Speaker"
        }
    ]"#;

    const SINK_INPUTS_FIXTURE: &str = r#"[
        {
            "corked": false,
            "mute": false,
            "volume": { "front-left": { "value_percent": "80%" } },
            "properties": { "application.name": "firefox", "media.name": "YouTube" }
        },
        {
            "corked": true,
            "mute": false,
            "volume": { "front-left": { "value_percent": "50%" } },
            "properties": { "application.name": "paused-app" }
        }
    ]"#;

    #[test]
    fn ic_vol_ramp_matches_volume_sh_thresholds() {
        assert_eq!(ic_vol(0, false), '\u{f057f}');
        assert_eq!(ic_vol(33, false), '\u{f057f}');
        assert_eq!(ic_vol(50, false), '\u{f0580}');
        assert_eq!(ic_vol(90, false), '\u{f057e}');
        assert_eq!(ic_vol(90, true), '\u{f075f}');
    }

    #[test]
    fn volume_class_shares_ic_vol_thresholds_plus_a_muted_bucket() {
        // T9: same 34/67 split as ic_vol()'s own ramp, but muted always
        // wins regardless of pct (mirrors ic_vol's own precedence).
        assert_eq!(volume_class(0, false), "quiet");
        assert_eq!(volume_class(33, false), "quiet");
        assert_eq!(volume_class(50, false), "mid");
        assert_eq!(volume_class(90, false), "loud");
        assert_eq!(volume_class(90, true), "muted");
        assert_eq!(volume_class(0, true), "muted");
    }

    #[test]
    fn dev_line_reads_volume_mute_label_port_profile() {
        let sinks: Value = serde_json::from_str(SINKS_FIXTURE).unwrap();
        let dev = dev_line(&sinks, "alsa_output.speaker").unwrap();
        assert_eq!(dev.pct, 40);
        assert!(!dev.muted);
        assert_eq!(dev.label, "Speaker");
        assert_eq!(dev.port, "Speaker");
        assert_eq!(dev.profile, "Speaker");
    }

    #[test]
    fn dev_line_returns_none_for_missing_default_or_unmatched_name() {
        let sinks: Value = serde_json::from_str(SINKS_FIXTURE).unwrap();
        assert!(dev_line(&sinks, "").is_none());
        assert!(dev_line(&sinks, "nonexistent").is_none());
    }

    #[test]
    fn app_lines_drops_corked_and_sorts_by_volume_descending() {
        let inputs: Value = serde_json::from_str(SINK_INPUTS_FIXTURE).unwrap();
        let apps = app_lines(&inputs);
        assert_eq!(apps.len(), 1, "the corked stream must be dropped");
        assert_eq!(apps[0].name, "firefox");
        assert_eq!(apps[0].title, "YouTube");
    }

    #[test]
    fn app_lines_defaults_name_to_question_mark_when_absent() {
        let json: Value = serde_json::from_str(
            r#"[{"corked":false,"mute":false,"volume":{"c":{"value_percent":"10%"}},"properties":{}}]"#,
        )
        .unwrap();
        assert_eq!(app_lines(&json)[0].name, "?");
    }

    #[test]
    fn detail_drops_entries_matching_label_or_each_other() {
        // All three equal ("Speaker" case) -> nothing left to show.
        assert_eq!(detail("Speaker", "Speaker", "Speaker"), None);
        // Port distinct from label, profile repeats port -> shown once.
        assert_eq!(
            detail("Speaker", "Headphones", "Headphones"),
            Some("Headphones".to_string())
        );
        // Both distinct -> joined in port, profile order.
        assert_eq!(
            detail("Speaker", "Headphones", "HiFi"),
            Some("Headphones  ·  HiFi".to_string())
        );
    }

    #[test]
    fn event_matches_filters_client_noise() {
        assert!(event_matches("Event 'change' on sink #59"));
        assert!(event_matches("Event 'new' on source #3"));
        assert!(event_matches("Event 'change' on server #0"));
        assert!(!event_matches("Event 'new' on client #42"));
    }

    #[test]
    fn ingest_line_dedups_raw_lines_and_filters_noise() {
        let mut a = Audio::new();
        assert!(a.ingest_line("Event 'change' on sink #1"));
        assert!(
            !a.ingest_line("Event 'change' on sink #1"),
            "identical line must not re-trigger"
        );
        assert!(!a.ingest_line("Event 'new' on client #2"));
        assert!(a.ingest_line("Event 'change' on source #1"));
    }

    #[test]
    fn build_tip_reports_no_audio_sink_verbatim() {
        // Exercised indirectly via refresh() in main.rs; build_tip() itself
        // is only called once a Dev exists (volume.sh:102-105's early exit
        // is mirrored in Audio::refresh, not here).
        let dev = Dev {
            pct: 40,
            muted: false,
            label: "Speaker".into(),
            port: "".into(),
            profile: "".into(),
        };
        let tip = build_tip(&dev, None, &[]);
        // T-popup-vert: "Volume" moved out of the body into its own
        // `vol_tip_title` ironvar (see genconfig.rs::popup()) — the body
        // now opens straight on the Output section.
        assert!(
            tip.trim_start().starts_with("<span"),
            "section markup expected: {tip}"
        );
        assert!(tip.contains("Output"));
        assert!(tip.contains("Playing"));
        assert!(!tip.contains("Microphone"), "no mic section without a Dev");
        assert!(tip.contains("nothing"), "empty Playing section");
        assert!(!tip.ends_with('\n'), "trailing newlines must be trimmed");
    }
}
