//! Generates ironbar's `config.json`. Corn (ironbar's own config language)
//! has no loops or includes, so 9 pills × N monitors would otherwise mean
//! either a shell templating layer (the naming scheme duplicated across two
//! languages, free to drift) or a checked-in per-monitor block (drifts the
//! moment a monitor is added or removed). Generating it here keeps
//! `mango::{ws_module, var_tags, var_ov, var_tip}` as the single source of
//! truth for every name this config and the collector must agree on — see
//! the tests at the bottom, which check that agreement mechanically.
//!
//! `install-config.sh` symlinks every regular file under a listed source
//! dir into the matching `~/.config/<dir>/`, so a generated
//! `~/.config/ironbar/config.json` must be a REAL file — the repo
//! deliberately contains nothing named `config.*` under `src/ironbar/` so
//! there is nothing for that symlink loop to collide with.

use crate::mango::{slugs_for, var_ov, var_tags, var_tip, ws_module, ws_module_ov, TAG_COUNT};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `mango-bard gen-config [--monitors a,b,...] [--out PATH|-]`
pub async fn main(args: &[String]) -> Result<(), String> {
    let mut monitors: Option<Vec<String>> = None;
    let mut out: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--monitors" => {
                let v = args
                    .get(i + 1)
                    .ok_or("gen-config: --monitors needs a value")?;
                monitors = Some(
                    v.split(',')
                        .map(String::from)
                        .filter(|s| !s.is_empty())
                        .collect(),
                );
                i += 2;
            }
            "--out" => {
                out = Some(
                    args.get(i + 1)
                        .ok_or("gen-config: --out needs a value")?
                        .clone(),
                );
                i += 2;
            }
            other => return Err(format!("gen-config: unknown argument {other}")),
        }
    }

    let monitors = match monitors {
        Some(m) => m,
        None => query_monitors().await?,
    };
    let text = serde_json::to_string_pretty(&build(&monitors)).map_err(|e| e.to_string())?;

    match out.as_deref() {
        Some("-") => println!("{text}"),
        Some(path) => write_atomic(Path::new(path), &text)?,
        None => write_atomic(&default_out_path(), &text)?,
    }
    Ok(())
}

fn default_out_path() -> PathBuf {
    let dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
    dir.join("ironbar").join("config.json")
}

async fn query_monitors() -> Result<Vec<String>, String> {
    let out = tokio::process::Command::new("mmsg")
        .args(["get", "all-monitors"])
        .output()
        .await
        .map_err(|e| format!("mmsg get all-monitors: {e}"))?;
    let doc: Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("bad all-monitors JSON: {e}"))?;
    Ok(doc
        .get("monitors")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("name").and_then(Value::as_str))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default())
}

/// `.tmp` + `rename`, same pattern bars.sh used for its generated config —
/// a reader never observes a half-written file.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Builds the whole ironbar config for `monitors`, in the order given.
/// `monitors` empty is valid — the top-level entry is also the fallback bar
/// for any output not named under `monitors` (T0 spike S3), so it still
/// renders clock/date/window with no workspace pills, matching S3's finding
/// that an unlisted output gets a bar rather than none at all.
pub fn build(monitors: &[String]) -> Value {
    let slugs = slugs_for(monitors);

    let mut defaults = serde_json::Map::new();
    defaults.insert("clk_text".into(), json!("--:--"));
    defaults.insert("date_text".into(), json!(""));
    defaults.insert("win_text".into(), json!(""));
    defaults.insert("win_tip".into(), json!(""));
    // T3 network vars — see net.rs. wifi_show defaults "true" (not busy) so
    // the pill is visible before the daemon's first regrade() lands.
    defaults.insert("net_busy".into(), json!("false"));
    defaults.insert("wifi_show".into(), json!("true"));
    defaults.insert("wifi_text".into(), json!(""));
    defaults.insert("eth_text".into(), json!(""));
    defaults.insert("sec_text".into(), json!(""));
    // T4 audio vars — see audio.rs.
    defaults.insert("vol_text".into(), json!(""));
    defaults.insert("vol_tip".into(), json!(""));
    defaults.insert("mic_text".into(), json!(""));
    // T5 power vars — see power.rs.
    defaults.insert("bat_text".into(), json!(""));
    defaults.insert("bat_tip".into(), json!(""));
    // T6a cpu/memory vars — see cpu.rs/memory.rs. No `_tip` entries yet: the
    // tooltip popups are T7's job.
    defaults.insert("cpu_text".into(), json!(""));
    defaults.insert("mem_text".into(), json!(""));
    for slug in &slugs {
        defaults.insert(var_tags(slug), json!("true"));
        defaults.insert(var_ov(slug), json!("false"));
        for tag in 1..=TAG_COUNT {
            defaults.insert(var_tip(slug, tag), json!(""));
        }
    }

    let mut monitors_map = serde_json::Map::new();
    for (name, slug) in monitors.iter().zip(&slugs) {
        let bar_name = format!("bar-{name}");
        monitors_map.insert(
            name.clone(),
            json!({
                "name": bar_name,
                "start": [ window_module() ],
                "center": workspace_pills(name, slug),
                "end": end_modules(&bar_name),
            }),
        );
    }

    json!({
        // Popups are click-driven (workspace pill right-click, window
        // left-click, volume left-click) — autohide is what makes clicking
        // away close them, since the default is `false`.
        "popup_autohide": true,
        "ironvar_defaults": Value::Object(defaults),
        // Named so the fallback bar's own volume popup has a
        // `toggle-popup` target (T0 spike S3: an unlisted output still gets
        // this bar). Two unlisted outputs would share the name — the
        // generator lists every real monitor, so that never happens today.
        "name": "bar-default",
        "start": [ window_module() ],
        "center": clock_pill(),
        "end": end_modules("bar-default"),
        "monitors": Value::Object(monitors_map),
    })
}

/// `end` row order follows config.jsonc:271 for volume/mic before the
/// network pills; cpu/memory (T6a) sit between network and battery, matching
/// waybar's own `group/leftcenter` order (config.jsonc:28: cpu, memory,
/// docker, battery, claudebar). Docker (T6b) and claudebar (T6c, deferred
/// behind T7) still have no anchor and will slot in here when they land.
fn end_modules(bar_name: &str) -> Vec<Value> {
    let mut end = audio_modules(bar_name);
    end.extend(net_modules());
    end.extend(cpu_modules());
    end.extend(power_modules(bar_name));
    end
}

fn window_module() -> Value {
    json!({
        "type": "custom",
        "name": "win",
        "bar": [ { "type": "label", "label": "#win_text" } ],
        "popup": [ { "type": "label", "label": "#win_tip" } ],
        "on_scroll_up": "brightnessctl set +5%",
        "on_scroll_down": "brightnessctl set 5%-"
    })
}

fn clock_pill() -> Vec<Value> {
    vec![json!({
        "type": "custom",
        "class": "pill",
        "bar": [
            { "type": "label", "name": "clock", "label": "#clk_text" },
            { "type": "label", "name": "date", "label": "#date_text" }
        ]
    })]
}

/// Single glyph, animated by CSS `@keyframes` — this is what T0/S4 already
/// concluded (spinner-over-IPC would cost ~72us x 8.3Hz forever while a
/// link transitions; a class-flipping var costs one set per transition
/// edge instead). First frame of net-watch.sh's own pie-slice family
/// (`\xf3\xb0\xaa\xa5`, U+F0AA5) — reused rather than picking a fresh glyph,
/// since it is already proven to render in this environment.
const NET_SPINNER_GLYPH: &str = "\u{f0aa5}";

/// Volume/mic pills: T4 (see audio.rs). Bar-global, like `net_modules()` —
/// audio state is the same on every monitor. Click layout is a T4 design
/// decision (IRONBAR.md has no hover tooltip, so the popup needs a click):
/// left opens the popup (the detail that used to live under hover), right
/// and middle keep waybar's own `on-click`/`on-click-right`
/// (config.jsonc:309-312) unchanged. Scroll wiring is copied verbatim from
/// the same lines.
fn audio_modules(bar_name: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "custom",
            "name": "volume",
            "class": "volume",
            "bar": [ { "type": "label", "label": "#vol_text" } ],
            "popup": [ { "type": "label", "label": "#vol_tip" } ],
            "on_click_left": format!("ironbar bar {bar_name} toggle-popup volume"),
            "on_click_right": "pavucontrol-qt",
            "on_click_middle": "pavucontrol-qt -t 5",
            "on_scroll_up": "wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%+ -l 1.0",
            "on_scroll_down": "wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%-"
        }),
        json!({
            "type": "custom",
            "name": "mic",
            "class": "mic",
            "bar": [ { "type": "label", "label": "#mic_text" } ]
        }),
    ]
}

/// Network pills: T3 (see net.rs). Bar-global — one instance per bar, like
/// `window_module()` — rather than per-monitor, since network state is the
/// same everywhere. Module names double as `@class/<module>` targets,
/// exactly as `ws_module()` establishes for workspace pills (mango.rs).
///
/// Click paths follow config.jsonc's existing convention for waybar's
/// on-click scripts (`~/.config/waybar/scripts/<script>`, confirmed by
/// grep against the live config) — T3 keeps every click-driven menu as
/// shell (IRONBAR.md goal 4); only the polling/event code moved into the
/// daemon. `nm-connection-editor` is a real installed binary, not a repo
/// script, so it is referenced bare.
fn net_modules() -> Vec<Value> {
    vec![
        json!({
            "type": "custom",
            "name": "net-spinner",
            "class": "net-spinner",
            "show_if": "#net_busy",
            "bar": [ { "type": "label", "label": NET_SPINNER_GLYPH } ]
        }),
        json!({
            "type": "custom",
            "name": "wifi",
            "class": "wifi",
            // Logical inverse of net_busy: the spinner REPLACES the wifi
            // pill rather than sitting beside it (net.sh:598-608's
            // contract, ported to show_if — see net.rs's refresh_busy_var).
            "show_if": "#wifi_show",
            "bar": [ { "type": "label", "label": "#wifi_text" } ],
            "on_click_left": "~/.config/waybar/scripts/wifi-menu.sh",
            "on_click_right": "nm-connection-editor"
        }),
        json!({
            "type": "custom",
            "name": "eth",
            "class": "eth",
            "bar": [ { "type": "label", "label": "#eth_text" } ],
            "on_click_left": "~/.config/waybar/scripts/eth-toggle.sh",
            "on_click_right": "nm-connection-editor"
        }),
        json!({
            "type": "custom",
            "name": "netsec",
            "class": "netsec",
            "bar": [ { "type": "label", "label": "#sec_text" } ],
            "on_click_left": "~/.config/waybar/scripts/net.sh --sec-click",
            "on_click_right": "~/.config/waybar/scripts/net.sh --sec-edit"
        }),
    ]
}

/// CPU/memory pills: T6a (see cpu.rs/memory.rs). Bar-global, like
/// `net_modules()` — one reading, not one per monitor. No `popup` yet (no
/// `_tip` var exists — T7 adds the tooltip content and the click migrates to
/// `toggle-popup`, same as `power_modules`/`audio_modules` already did).
/// `on_click_left` keeps waybar's own click (config.jsonc:35,42): opening
/// btop, since there is no popup to open yet.
fn cpu_modules() -> Vec<Value> {
    vec![
        json!({
            "type": "custom",
            "name": "cpu",
            "class": "cpu",
            "bar": [ { "type": "label", "label": "#cpu_text" } ],
            "on_click_left": "kitty --class mango-monitor -e btop"
        }),
        json!({
            "type": "custom",
            "name": "memory",
            "class": "memory",
            "bar": [ { "type": "label", "label": "#mem_text" } ],
            "on_click_left": "kitty --class mango-monitor -e btop"
        }),
    ]
}

/// Battery pill: T5 (see power.rs). Bar-global, like `net_modules()` — one
/// battery, not one per monitor. Click layout follows T4's rule (no hover
/// tooltip exists in ironbar, so rich detail needs a click): left opens the
/// popup, replacing waybar's own plain left-click; middle takes over
/// waybar's old left (`powermode.sh toggle`, config.jsonc:249); right keeps
/// waybar's own right-click powertop report unchanged.
fn power_modules(bar_name: &str) -> Vec<Value> {
    vec![json!({
        "type": "custom",
        "name": "battery",
        "class": "battery",
        "bar": [ { "type": "label", "label": "#bat_text" } ],
        "popup": [ { "type": "label", "label": "#bat_tip" } ],
        "on_click_left": format!("ironbar bar {bar_name} toggle-popup battery"),
        "on_click_middle": "~/.config/mango/scripts/powermode.sh toggle",
        "on_click_right": "kitty --class mango-monitor -e sudo -n powertop"
    })]
}

/// Nine numbered pills plus one overview pill for `mon`. Each pill is a
/// `custom` module (not a widget) because `style add-class`/`remove-class`
/// match module names, and `popup` exists only on `custom` modules.
///
/// `on_click_left`/`on_click_right` are module-level `ScriptInput`, which
/// runs its string as a plain shell command — unlike a *widget*-level
/// `on_click`, which is overloaded between built-in actions
/// (`popup:toggle`) and a shell command disambiguated by a leading `!`.
/// Verified live (T2 probe V1): no `!` prefix here.
fn workspace_pills(mon: &str, slug: &str) -> Vec<Value> {
    let bar_name = format!("bar-{mon}");
    let mut pills: Vec<Value> = (1..=TAG_COUNT)
        .map(|n| {
            let module = ws_module(mon, n);
            json!({
                "type": "custom",
                "name": module,
                "class": "ws",
                "show_if": format!("#{}", var_tags(slug)),
                "bar": [ { "type": "label", "label": n.to_string() } ],
                "popup": [ { "type": "label", "label": format!("#{}", var_tip(slug, n)) } ],
                "on_click_left": format!("mmsg dispatch view,{n},0"),
                "on_click_right": format!("ironbar bar {bar_name} toggle-popup {module}"),
                "on_scroll_up": "mmsg dispatch viewtoleft,0",
                "on_scroll_down": "mmsg dispatch viewtoright,0"
            })
        })
        .collect();

    // Overview is its own module rather than pill 1's old dual-purpose
    // state (workspace.sh --click's toggleoverview/view,1,0 branch): with
    // `show_if` able to hide the numbered pills outright, nothing needs to
    // ask "am I in overview?" at click time — each module gets a static
    // on_click_left and the two are simply never visible together.
    pills.push(json!({
        "type": "custom",
        "name": ws_module_ov(mon),
        "class": "ws-overview",
        "show_if": format!("#{}", var_ov(slug)),
        "bar": [ { "type": "label", "label": "overview" } ],
        "on_click_left": "mmsg dispatch toggleoverview,"
    }));
    pills.extend(clock_pill());
    pills
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn collect_var_refs(v: &Value, out: &mut HashSet<String>) {
        match v {
            Value::String(s) => {
                if let Some(name) = s.strip_prefix('#') {
                    out.insert(name.to_string());
                }
            }
            Value::Array(a) => a.iter().for_each(|x| collect_var_refs(x, out)),
            Value::Object(m) => m.values().for_each(|x| collect_var_refs(x, out)),
            _ => {}
        }
    }

    fn collect_module_names(v: &Value, out: &mut HashSet<String>) {
        match v {
            Value::Object(m) => {
                if let Some(Value::String(n)) = m.get("name") {
                    out.insert(n.clone());
                }
                m.values().for_each(|x| collect_module_names(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| collect_module_names(x, out)),
            _ => {}
        }
    }

    #[test]
    fn every_referenced_var_has_a_default() {
        let cfg = build(&["eDP-1".to_string(), "DP-1".to_string()]);
        let mut refs = HashSet::new();
        collect_var_refs(&cfg, &mut refs);
        let defaults: HashSet<String> = cfg["ironvar_defaults"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        for r in &refs {
            assert!(
                defaults.contains(r),
                "no ironvar_defaults entry for referenced #{r}"
            );
        }
    }

    #[test]
    fn every_pill_module_name_agrees_with_mango_ws_module() {
        let cfg = build(&["eDP-1".to_string()]);
        let mut names = HashSet::new();
        collect_module_names(&cfg, &mut names);
        for tag in 1..=TAG_COUNT {
            assert!(names.contains(&ws_module("eDP-1", tag)));
        }
        assert!(names.contains(&ws_module_ov("eDP-1")));
    }

    #[test]
    fn toggle_popup_targets_the_monitors_own_bar_name() {
        let cfg = build(&["eDP-1".to_string()]);
        assert_eq!(cfg["monitors"]["eDP-1"]["name"], json!("bar-eDP-1"));
        let click = cfg["monitors"]["eDP-1"]["center"][0]["on_click_right"]
            .as_str()
            .unwrap();
        assert!(click.contains("bar-eDP-1"));
        assert!(
            !click.starts_with('!'),
            "module-level ScriptInput needs no ! prefix"
        );
    }

    #[test]
    fn volume_popup_targets_its_own_bar_name() {
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let volume = end.iter().find(|m| m["name"] == "volume").unwrap();
        assert!(volume["on_click_left"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));

        // The fallback bar (no monitor listed) must also get a name to
        // target, per T0 spike S3 — an unlisted output still gets this bar.
        let fallback = build(&[]);
        assert_eq!(fallback["name"], json!("bar-default"));
        let fb_end = fallback["end"].as_array().unwrap();
        let fb_volume = fb_end.iter().find(|m| m["name"] == "volume").unwrap();
        assert!(fb_volume["on_click_left"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    #[test]
    fn battery_popup_targets_its_own_bar_name() {
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let battery = end.iter().find(|m| m["name"] == "battery").unwrap();
        assert!(battery["on_click_left"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));

        let fallback = build(&[]);
        let fb_end = fallback["end"].as_array().unwrap();
        let fb_battery = fb_end.iter().find(|m| m["name"] == "battery").unwrap();
        assert!(fb_battery["on_click_left"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    #[test]
    fn ironvar_keys_are_pure_ascii_alphanumeric_or_underscore() {
        let cfg = build(&["eDP-1".to_string(), "HDMI-A-1".to_string()]);
        for key in cfg["ironvar_defaults"].as_object().unwrap().keys() {
            assert!(
                key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "bad key: {key}"
            );
        }
    }

    #[test]
    fn empty_monitor_list_still_builds_a_fallback_bar() {
        let cfg = build(&[]);
        assert!(cfg["monitors"].as_object().unwrap().is_empty());
        assert!(cfg["center"].as_array().is_some());
    }

    #[test]
    fn cpu_and_memory_sit_between_net_and_battery() {
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let netsec = names.iter().position(|n| *n == "netsec").unwrap();
        let cpu = names.iter().position(|n| *n == "cpu").unwrap();
        let memory = names.iter().position(|n| *n == "memory").unwrap();
        let battery = names.iter().position(|n| *n == "battery").unwrap();
        assert!(netsec < cpu && cpu < memory && memory < battery);
    }
}
