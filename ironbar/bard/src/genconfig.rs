//! Generates ironbar's `config.json`. Corn (ironbar's own config language)
//! has no loops or includes, so 9 pills × N monitors would otherwise mean
//! either a shell templating layer (the naming scheme duplicated across two
//! languages, free to drift) or a checked-in per-monitor block (drifts the
//! moment a monitor is added or removed). Generating it here keeps
//! `mango::{ws_module, var_tags, var_ov, var_tip, var_lbl}` as the single
//! source of truth for every name this config and the collector must agree
//! on — see the tests at the bottom, which check that agreement
//! mechanically.
//!
//! `install-config.sh` symlinks every regular file under a listed source
//! dir into the matching `~/.config/<dir>/`, so a generated
//! `~/.config/ironbar/config.json` must be a REAL file — the repo
//! deliberately contains nothing named `config.*` under `src/ironbar/` so
//! there is nothing for that symlink loop to collide with.

use crate::cmd::CMD_TIMEOUT;
use crate::mango::{
    overview_label, slugs_for, tag_label, var_lbl, var_ov, var_tags, var_tip, ws_module,
    ws_module_ov, TAG_COUNT,
};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Builds a popup's single top-level widget for `tip_key` (a bare ironvar
/// name, no leading `#`): a vertical `box` holding, in order, an optional
/// title label, a body label bound to the ironvar, and an optional hint
/// footer — each block separated by a real `popup-sep` box.
///
/// T-popup-vert: ironbar's own popup container (built from the `popup:`
/// array) is a hardcoded `gtk::Box::new(Orientation::Horizontal, 0)`
/// (`src/modules/custom/mod.rs::into_popup`, confirmed against the vendored
/// ironbar source) — every top-level widget in `popup:` becomes its
/// horizontal sibling. That is why the previous three-sibling shape (body,
/// sep, hint) packed the hint to the body's right instead of below it: a
/// `min-height: 1px` box with no `min-width` in a horizontal layout also
/// collapses to nothing, so there was no visible divider either. Handing
/// `popup:` exactly one widget — this `box` with `orientation: "vertical"`
/// — moves the stacking into `BoxWidget::into_widget`, the one config-level
/// box type that actually honours `orientation`. Both `popup-sep` boxes are
/// then real siblings inside the same vertical box, so both span its full
/// content width and are identical to each other by construction — no
/// per-caller width number needed (see `tooltip::set_titled`'s own doc
/// comment for what that replaces).
///
/// T-popup-hold: the outer box also carries `on_mouse_enter`/
/// `on_mouse_exit` sending `hover hold`/`hover release` — confirmed against
/// the vendored ironbar source that these fire on *any* widget, not just a
/// bar-level module (`common.rs::install_events` runs for every
/// `WidgetConfig`, including nested popup widgets — see
/// `main.rs::hover_hold`/`hover_release`'s own doc comments for the state
/// they arm/cancel). Wired for every popup, including `win_tip`'s (opened
/// by click, not hover, so its own `hover_open` entry never exists) —
/// harmless there: `hover_hold`/`hover_release` both cross-check `bar`
/// against whatever `hover_open` already holds and no-op on a mismatch, so
/// the worst case is a no-op rather than a wrong `hide_popup` call.
fn popup(tip_key: &str, bar_name: &str) -> Value {
    let mut widgets = Vec::new();
    if crate::tooltip::TITLED.contains(&tip_key) {
        widgets.push(
            json!({ "type": "label", "class": "popup-title", "label": format!("#{tip_key}_title") }),
        );
        widgets.push(json!({ "type": "box", "class": "popup-sep" }));
    }
    widgets.push(json!({ "type": "label", "label": format!("#{tip_key}") }));
    if let Some((_, hint)) = crate::tooltip::HINTS.iter().find(|(k, _)| *k == tip_key) {
        widgets.push(json!({ "type": "box", "class": "popup-sep" }));
        widgets.push(json!({ "type": "label", "class": "popup-hint", "label": hint }));
    }
    json!([{
        "type": "box",
        "orientation": "vertical",
        "widgets": widgets,
        "on_mouse_enter": format!("mango-bard hover hold {bar_name} -q"),
        "on_mouse_exit": format!("mango-bard hover release {bar_name} -q")
    }])
}

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
    let out = tokio::time::timeout(
        CMD_TIMEOUT,
        tokio::process::Command::new("mmsg")
            .args(["get", "all-monitors"])
            .output(),
    )
    .await
    .map_err(|_| "mmsg get all-monitors: timed out".to_string())?
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
    // T-next clock/date popup vars — see clock.rs's refresh_clock_tip/
    // refresh_date_tip.
    defaults.insert("clk_tip".into(), json!(""));
    defaults.insert("date_tip".into(), json!(""));
    // T7c pomodoro vars — see pomo.rs. No `_show`: unlike hotspot's
    // hide-until-active pill, the pomodoro pill is always visible (an idle
    // glyph is still useful — it's the click target that starts a block).
    defaults.insert("pomo_text".into(), json!(""));
    defaults.insert("pomo_tip".into(), json!(""));
    defaults.insert("win_text".into(), json!(""));
    defaults.insert("win_tip".into(), json!(""));
    // T3 network vars — see net.rs. wifi_show defaults "true" (not busy) so
    // the pill is visible before the daemon's first regrade() lands.
    defaults.insert("net_busy".into(), json!("false"));
    defaults.insert("wifi_show".into(), json!("true"));
    defaults.insert("wifi_text".into(), json!(""));
    defaults.insert("eth_text".into(), json!(""));
    defaults.insert("sec_text".into(), json!(""));
    // T19: wifi/eth/netsec detail popups (T7b) — see net.rs's
    // refresh_wifi_tip/refresh_eth_tip/refresh_sec_tip.
    defaults.insert("wifi_tip".into(), json!(""));
    defaults.insert("eth_tip".into(), json!(""));
    defaults.insert("sec_tip".into(), json!(""));
    // T4 audio vars — see audio.rs.
    defaults.insert("vol_text".into(), json!(""));
    defaults.insert("vol_tip".into(), json!(""));
    defaults.insert("mic_text".into(), json!(""));
    // T5 power vars — see power.rs.
    defaults.insert("bat_text".into(), json!(""));
    defaults.insert("bat_tip".into(), json!(""));
    // T6a cpu/memory vars — see cpu.rs/memory.rs. `_tip` entries added at
    // T7a: the popup content, built lazily by `refresh_detail` (D1/D2).
    defaults.insert("cpu_text".into(), json!(""));
    defaults.insert("cpu_tip".into(), json!(""));
    defaults.insert("mem_text".into(), json!(""));
    defaults.insert("mem_tip".into(), json!(""));
    // T6b docker/hotspot/darkmode vars — see docker.rs/hotspot.rs/
    // darkmode.rs. hotspot_show defaults "false": hidden until proven active
    // (IRONBAR.md parity table — waybar's own pill is empty-text while down
    // too), unlike wifi_show's "true" default above.
    defaults.insert("docker_text".into(), json!(""));
    defaults.insert("docker_tip".into(), json!(""));
    defaults.insert("hotspot_show".into(), json!("false"));
    defaults.insert("hotspot_text".into(), json!(""));
    defaults.insert("hotspot_tip".into(), json!(""));
    // remote (wayvnc + kdeconnectd) — see remote.rs. No `_show`: this pill
    // has no show_if, unlike hotspot's above.
    defaults.insert("remote_text".into(), json!(""));
    defaults.insert("remote_tip".into(), json!(""));
    // keep-awake — see keepawake.rs. Keys stay named `inhibit_*` (not
    // `keepawake_*`): they replace the native `inhibit` module's own
    // `format_on`/`format_off` in place, and the style.css `.inhibit`
    // selectors already target this class.
    defaults.insert("inhibit_text".into(), json!(""));
    defaults.insert("inhibit_tip".into(), json!(""));
    defaults.insert("dark_icon".into(), json!(""));
    // T6c claudebar vars — see claude.rs.
    defaults.insert("claude_text".into(), json!(""));
    defaults.insert("claude_tip".into(), json!(""));
    // T-popup-vert: every TITLED key gets a matching `<key>_title` default,
    // looped from that one table rather than a second insert beside each
    // key above, so the two lists can't drift apart — see
    // `tooltip::set_titled`'s own doc comment.
    for key in crate::tooltip::TITLED {
        defaults.insert(format!("{key}_title"), json!(""));
    }
    for slug in &slugs {
        defaults.insert(var_tags(slug), json!("true"));
        defaults.insert(var_ov(slug), json!("false"));
        for tag in 1..=TAG_COUNT {
            defaults.insert(var_tip(slug, tag), json!(""));
            // T-next: the dots-under-number label var — see mango.rs's
            // `tag_label`. T20: default is the real zero-window label, not
            // a bare number — the label block must have the same height
            // before and after the daemon's first `apply()`, or the row
            // jumps on startup.
            defaults.insert(var_lbl(slug, tag), json!(tag_label(tag, 0)));
        }
    }

    let mut monitors_map = serde_json::Map::new();
    for (name, slug) in monitors.iter().zip(&slugs) {
        let bar_name = format!("bar-{name}");
        // T-next (item 3): `end` is `rightcenter_modules()` (the old
        // clock/pomo/colorpicker/darkmode/snip/inhibit tail of `center`)
        // followed by this bar's own tray/audio/net/hotspot/bluetooth/power
        // — see `rightcenter_modules()`'s own doc comment for why that
        // group moved here instead of staying in `center`.
        let mut end = rightcenter_modules(&bar_name);
        end.extend(end_modules(&bar_name));
        monitors_map.insert(
            name.clone(),
            json!({
                "name": bar_name,
                // T8c: = config.jsonc:4's `"height": 40`. Ironbar's own
                // default is 42 (schema default, `BarConfig.height`), and
                // GTK treats it as a minimum, not a fixed value — it grows
                // to fit taller content regardless. Set for parity with
                // waybar's own height, not as a hard cap.
                "height": 40,
                // T8 cutover: ironbar takes the top edge waybar used to own
                // (bars.sh killed, not restarted). No separate popup-
                // direction setting exists (`--print-schema` confirmed at
                // T0/S5) — ironbar derives it from `position` itself.
                "position": "top",
                // T9: raised from the schema default (5) — the only distance
                // lever `PopupConfig`/`BarConfig` expose for "popup sits too
                // close to the button" (no per-widget offset field exists).
                "popup_gap": 12,
                "start": start_modules(&bar_name),
                // T-next (item 3): the 9 workspace pills, and nothing else
                // — see `workspace_pills()`'s own doc comment for why this
                // slot used to hold 15 modules and now holds only these.
                "center": workspace_pills(name, slug),
                "end": end,
            }),
        );
    }

    // No workspace pills on the fallback bar (no monitor name/slug to build
    // them from — T0 spike S3's unlisted-output case), so `rightcenter_
    // modules()` keeps its old position inside `center` here instead of
    // moving to `end` the way it does for a real per-monitor bar above —
    // there is no pill group on this bar for it to visually protect.
    let fallback_center = rightcenter_modules("bar-default");

    json!({
        // Popups are click-driven (workspace pill right-click, window
        // left-click, volume left-click) — autohide is what makes clicking
        // away close them, since the default is `false`.
        "popup_autohide": true,
        // T9: see the per-monitor bar's own "popup_gap" comment above — same
        // reasoning, applied to the fallback bar too.
        "popup_gap": 12,
        "ironvar_defaults": Value::Object(defaults),
        // Named so the fallback bar's own volume popup has a
        // `toggle-popup` target (T0 spike S3: an unlisted output still gets
        // this bar). Two unlisted outputs would share the name — the
        // generator lists every real monitor, so that never happens today.
        "name": "bar-default",
        "height": 40,
        "position": "top",
        "start": start_modules("bar-default"),
        "center": fallback_center,
        "end": end_modules("bar-default"),
        "monitors": Value::Object(monitors_map),
    })
}

/// `end` row: config.jsonc:271's own `group/indicators` order — volume/mic,
/// the network pills, then hotspot/bluetooth/power. T8d moved cpu/memory/
/// docker/battery/claudebar/music OUT of this row and into `center`'s
/// `leftcenter_modules()`, to match waybar's actual layout (those six render
/// at the far left of the center row, not the right end) — see IRONBAR.md's
/// T8d entry for the correction. T-next (item 3): `center` itself moved
/// again, to `start` — see `leftcenter_modules()`'s own doc comment.
///
/// T9: `tray` now leads this row, matching waybar's own `modules-right =
/// [tray, group/indicators]` (config.jsonc:9) — tray sits at the left edge
/// of the right-hand region, ahead of the indicator pills, not merged into
/// them. T8a had put tray in `start` specifically to dodge the box-nesting
/// popup-addressing risk (Phase 2 finding), but `TrayModule`'s schema has no
/// `popup`/`PopupConfig` field at all — that risk never applied to tray, so
/// it can sit flat in `end` like every other T8d module. Known, accepted
/// cosmetic deviation from waybar: it now shares `#bar #end`'s pill
/// background (waybar's own `#tray` rule is unstyled) — not worth
/// re-opening the box-nesting risk to avoid one shared background color,
/// same call T8a already made for the rest of this row.
///
/// `build()` prepends [`rightcenter_modules`] to this on every per-monitor
/// bar — see that function's own doc comment for why tray doesn't lead the
/// visible row even though it leads this function's own return value.
fn end_modules(bar_name: &str) -> Vec<Value> {
    let mut end = vec![tray_module()];
    end.extend(audio_modules(bar_name));
    end.extend(net_modules(bar_name));
    end.push(hotspot_module(bar_name));
    end.push(remote_module(bar_name));
    end.push(bluetooth_module(bar_name));
    end.push(power_module());
    end
}

/// `start`'s trailing block — waybar's own `group/leftcenter` order
/// (config.jsonc:26-29: cpu, memory, docker, battery, claudebar, mpris),
/// appended to `start` after the launcher/window title. `music` takes
/// waybar's `mpris` slot and `claudebar` is T6c's new pill, in waybar's own
/// slot between battery and mpris/music.
///
/// T-next (item 3): moved here from leading `center`. `center` used to hold
/// this group plus the 9 workspace pills plus [`rightcenter_modules`] — 15
/// modules of mixed, content-dependent width, of which only the 9 pills are
/// what a viewer reads as "the centre". GTK centres whatever `center`
/// contains as one block, so any of the other 6 changing width (a longer
/// pomodoro countdown, a music title, a claude-usage percentage) visibly
/// shifted the pills sideways. `center` now holds only
/// [`workspace_pills`] — see that function's own doc comment — so this
/// group needed a new home; `start` already reads left-to-right before the
/// pills, same position it held inside the old `center`.
fn leftcenter_modules(bar_name: &str) -> Vec<Value> {
    let mut m = cpu_modules(bar_name);
    m.push(docker_module(bar_name));
    m.extend(power_modules(bar_name));
    m.push(claudebar_module(bar_name));
    m.push(music_module());
    m
}

/// The group that used to trail the workspace pills inside `center`
/// (T8a's own `group/rightcenter`: clock, date, colorpicker, darkmode,
/// snip, idle_inhibitor — config.jsonc:186). `build()` prepends this to
/// [`end_modules`] on every per-monitor bar, so the group keeps its old
/// position immediately right of the pills — `center` no longer carries it
/// (see [`workspace_pills`]'s own doc comment for why), and `end` is the
/// next slot to its right in reading order, same as before the move.
fn rightcenter_modules(bar_name: &str) -> Vec<Value> {
    let mut m = clock_pill(bar_name);
    m.push(pomo_pill(bar_name));
    m.push(colorpicker_module());
    m.push(darkmode_module());
    m.push(snip_module());
    m.push(inhibit_module(bar_name));
    m
}

/// T8c: `truncate` added (= config.jsonc:22's `max-length: 32`, dropped
/// when this module was first written) — an unbounded window title/appid
/// was one of three causes found this stage for `power` being pushed off
/// the right edge of the screen; see `music_module()` and `build()`'s
/// `"height"` for the other two.
///
/// T-popup-fix: bar widget is a `button` wrapping a nested `label`, not a
/// bare `label` — see the module doc comment at the top of this file's
/// popup-fix stage for why every popup-bearing module needs this. `truncate`
/// has no `ButtonWidget` field (only `LabelWidget` has one), so the label
/// stays nested under `widgets` rather than moving to the button's own
/// `label` shorthand.
fn window_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "win",
        "class": "win",
        "bar": [ { "type": "button", "widgets": [
            { "type": "label", "label": "#win_text", "truncate": { "mode": "end", "max_length": 32 } }
        ] } ],
        "popup": popup("win_tip", bar_name),
        "tooltip": "Focused window — scroll to adjust brightness",
        "on_scroll_up": "brightnessctl set +5%",
        "on_scroll_down": "brightnessctl set 5%-"
    })
}

/// T8a: static click-only buttons and native window title, ported from
/// config.jsonc's `modules-left` (`custom/spark`, `custom/window`).
/// T9: `tray` moved OUT of this row and into `end_modules()`'s front — see
/// that function's own doc comment for why.
///
/// T-next (item 3): [`leftcenter_modules`] appended here — see that
/// function's own doc comment for why cpu/memory/docker/battery/claudebar/
/// music moved out of `center`. `start` no longer matches waybar's own
/// `modules-left` 1:1 (waybar kept that group in its own centre row); this
/// bar's `center` is reserved for the workspace pills alone.
fn start_modules(bar_name: &str) -> Vec<Value> {
    let mut m = vec![spark_module(), window_module(bar_name)];
    m.extend(leftcenter_modules(bar_name));
    m
}

/// Launcher button — config.jsonc:11-15, static, no daemon var.
///
/// T8b: U+E65F (Material Symbols "auto_awesome") replaced with U+F005
/// (star, JetBrainsMono Nerd Font) — see the T8b entry in IRONBAR.md for why
/// every Material Symbols Rounded codepoint on this bar was swapped for a
/// Nerd Font equivalent: GTK4 cannot correctly rasterize this variable font
/// on this system (confirmed with pango-view, which renders it fine outside
/// GTK4), so any Material Symbols glyph here draws a wrong CJK substitute.
///
/// T15 (superseded by T17): the font is not the gap — confirmed with
/// `fonttools`' `getBestCmap()` against the exact installed file
/// (`JetBrainsMonoNerdFont-Regular.ttf`), which lists TWO correctly-named,
/// correctly-mapped Arch Linux glyphs: `linux-archlinux` at U+F303 and
/// `dev-archlinux` at U+E732. Both render the actual Arch logo under
/// `pango-view --font="JetBrainsMono Nerd Font 24"`, outside GTK4 — yet
/// both failed live (U+F303 a bare dash, U+E732 a CJK glyph), and T15
/// concluded GTK4 could not draw them.
///
/// T17 found the real cause, and it is not GTK4 rasterization: the CSS
/// `*` rule matches every node, including each label, and in GTK4 CSS a
/// direct match beats an inherited value — so the `.spark` pill rule's
/// Nerd Font stack never reached any label. Every icon rendered through
/// fontconfig per-character fallback. Most PUA codepoints fell through to
/// a plausible glyph, which is what made this bug look codepoint-specific.
/// U+F303/U+E732 are claimed by IBM Plex Sans TC (legacy CJK PUA mapping)
/// which outranks JetBrains in the fallback sort: the "bare dash" was the
/// CJK ideograph 一, the "tofu" a CJK glyph, both from Plex TC. T15's
/// pango-view checks missed it because they forced the Nerd Font — a
/// stack no label actually had. Fixed in style.css by adding `label`
/// descendant selectors to the pill rule (class specificity beats `*`);
/// verified live via `ironbar var set` glyph injection plus a minimal
/// GTK4 inherit-vs-direct repro. With the font stack actually applied,
/// U+F303 draws the Arch logo — the T8b/T15 launcher request is met.
fn spark_module() -> Value {
    json!({
        "type": "custom",
        "name": "spark",
        "class": "spark",
        "bar": [ { "type": "label", "label": "\u{f303}" } ],
        "tooltip": "Launcher",
        "on_click_left": "rofi -show drun"
    })
}

/// Native tray module — the reported missing tray icons (genconfig.rs had
/// no `tray` module at all before T8a). `icon_size: 16` (T8d) is ironbar's
/// own schema default already, but stated explicitly since it is the one
/// compression knob this module exposes — the rest of the tray's width
/// comes from GTK4's default button padding on each `.tray .item`, closed
/// in style.css instead (no config-side spacing/gap option exists on this
/// module — `ironbar --print-schema` confirmed it). T9: moved from `start`
/// to lead `end` — see `end_modules()`'s doc comment.
fn tray_module() -> Value {
    json!({
        "type": "tray",
        "name": "tray",
        "class": "tray",
        "icon_size": 16
    })
}

/// T8c: split into two NAMED `custom` modules — the single unnamed module
/// this used to be (one module, two labels) is why the clock and date ran
/// together with no visible separator (`13:50Sat, 22 Aug`) and, separately,
/// why `ipc.rs::set_class` could never reach it: `@class/`-prefixed dirty
/// tracking addresses by module `name`, and this module had none. `clk_text`
/// gains its own `.clock`/`.date` state class this way, once clock.rs starts
/// setting one (T7c's pomodoro `work`/`paused` classes, ported from
/// waybar/style.css:327-334, need exactly this). The separator itself moved
/// into `clk_text`'s/`date_text`'s own markup (clock.rs), not the config —
/// see clock.rs's own comment.
///
/// T-next: both gain a popup (T7c-rest, see clock.rs's `refresh_clock_tip`/
/// `refresh_date_tip`) — click follows T7a's refresh-then-toggle-popup
/// pattern (cpu_modules' own doc comment explains why a plain `on_click`
/// can't be a combined array). Date's click launches the calendar app
/// (T15: moved from middle- to left-click, middle-click retired bar-wide),
/// carried over from waybar's own date left-click (clock.sh's own
/// `--calendar` action, unchanged — T8 owns extracting it into a
/// click-only script, same as `wifi-menu.sh` for the net pills).
fn clock_pill(bar_name: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "custom",
            "name": "clock",
            "class": "clock pill",
            "bar": [ { "type": "button", "label": "#clk_text" } ],
            "popup": popup("clk_tip", bar_name),
            // T13: static `tooltip` deleted — it fired on the same hover as
            // the popup and competed for the same space (genconfig.rs's
            // `tooltip_and_hover_are_mutually_exclusive` test enforces
            // this). See tooltip.rs's `HINTS` for where any gesture hint a
            // deleted tooltip used to carry moved to.
            //
            // Correction, T-hover follow-up: `on_click_left`'s
            // refresh-then-toggle-popup is gone, same reasoning as
            // claudebar_module()'s own doc comment — a plain detail popup
            // with no mutating click has nothing left for a click to do
            // once hover owns opening it, and leaving the toggle in place
            // let a click undo what hover just opened (`ipc.rs::show_popup`
            // sends `toggle_popup` under the hood — see its doc comment).
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} clock -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} clock -q")
        }),
        json!({
            "type": "custom",
            "name": "date",
            "class": "date pill",
            "bar": [ { "type": "button", "label": "#date_text" } ],
            "popup": popup("date_tip", bar_name),
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} date -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} date -q"),
            // T15: left-click, not middle — middle-click retired from every
            // module on this bar.
            "on_click_left": "~/.config/ironbar/scripts/clock.sh --calendar"
        }),
    ]
}

/// T7c pomodoro pill (see pomo.rs). One glyph plus icon; state (idle/work/
/// short/long/paused) is a CSS class (`@class/pomo`), not a second module —
/// same reasoning `clock_pill()`'s own doc comment gives for putting state
/// on the class rather than duplicating modules per state. Always visible,
/// unlike `hotspot_module()`'s `show_if`-hidden-until-active pill: an idle
/// pomodoro glyph is still a click target ("start a focus block"), not dead
/// weight to hide.
///
/// Click layout follows T4's rule (cpu_modules' own doc comment): left opens
/// the popup — and, while idle, `pomo click`'s own idle branch spawns
/// focus-task.sh first, so naming a task and viewing the (then-started)
/// block's detail happen from the same click. Left is folded from
/// clock.sh's own pause/resume gesture (idle -> start via focus-task.sh,
/// otherwise -> pause/resume), so nothing clock.sh offered is lost.
///
/// T15: middle-click retired bar-wide. Right now mutes (ported from
/// clock.sh:396-423's own middle-click) — reset has no click left to sit on
/// and drops off the bar; `mango-bard pomo reset -q` is still reachable from
/// the CLI, just not from a bar gesture.
/// T-hover: `on_mouse_enter`/`on_mouse_exit` added for a passive peek —
/// `pomo` is a partial case, not a plain copy of the other hover modules'
/// treatment. `on_click_left` is `pomo click`, which both starts/pauses the
/// timer (a mutating action) *and* opens the popup; hover must never trigger
/// a mutating action, so `on_click_left`/`on_click_right` are left
/// completely untouched here. Hover-open instead calls `pomo.refresh()`
/// directly (main.rs's hover select arm) — the same pure, non-mutating
/// render `on_minute`/`catch_up`/every `control()` branch already uses to
/// redraw `pomo_tip`, never the mutating `click`/`toggle`/`mute` verb.
fn pomo_pill(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "pomo",
        "class": "pomo pill",
        "bar": [ { "type": "button", "label": "#pomo_text" } ],
        "popup": popup("pomo_tip", bar_name),
        // Correction, T-hover follow-up: `on_click_left` used to also
        // toggle the popup (`; ironbar bar {bar} toggle-popup pomo`) — now
        // dropped. `ipc.rs::show_popup` sends `toggle_popup` under the hood
        // (see its doc comment; `ironbar`'s own `show_popup` command
        // rejects every `custom` module), so a click landing after hover
        // already opened the popup would toggle it closed again. The click
        // still only ever needs to mutate state (start/pause); hover alone
        // owns showing and hiding the popup, same as every other converted
        // pill.
        "on_click_left": "mango-bard pomo click -q",
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} pomo -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} pomo -q"),
        "on_click_right": "mango-bard pomo mute -q"
    })
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
///
/// T9: `vol_text` dropped its `NN%` suffix (see `audio.rs::refresh()` —
/// icon only now, percentage stays in the popup this already opens on
/// click) to stop crowding the wifi/eth/netsec trio next to it. A
/// `SliderWidget` in the popup was tried and rejected: live-spiked against
/// a throwaway ironbar instance (`SliderWidget.value`'s `ScriptInput`),
/// its value script fired 8 times *before the popup was ever opened* and
/// kept firing roughly every 3s with the popup closed — an unconditional
/// poll, not a fetch-on-open, the exact always-on pattern T7a's own design
/// correction already rejected for embedded interval scripts (IRONBAR.md's
/// T7a entry). No slider; the text meter `tooltip.rs` already builds stays
/// the only popup content.
fn audio_modules(bar_name: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "custom",
            "name": "volume",
            "class": "volume",
            "bar": [ { "type": "button", "label": "#vol_text" } ],
            "popup": popup("vol_tip", bar_name),
            // T-hover: on_click_left's plain toggle-popup is gone — hover
            // opens/closes it instead (marks audio_due, same "already fresh
            // off its own event stream" shape as battery/docker). Scroll and
            // right are untouched; T15 moves the tabbed mixer shortcut that
            // used to live on middle-click onto left (freed by the toggle
            // removal above — a launch, not a toggle, so it cannot fight
            // hover's open/close).
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} volume -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} volume -q"),
            "on_click_left": "pavucontrol-qt -t 5",
            "on_click_right": "pavucontrol-qt",
            "on_scroll_up": "wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%+ -l 1.0",
            "on_scroll_down": "wpctl set-volume @DEFAULT_AUDIO_SINK@ 2%-"
        }),
        json!({
            "type": "custom",
            "name": "mic",
            "class": "mic",
            "bar": [ { "type": "label", "label": "#mic_text" } ],
            "tooltip": "Microphone mute state"
        }),
    ]
}

/// Network pills: T3 (see net.rs). Bar-global — one instance per bar, like
/// `window_module()` — rather than per-monitor, since network state is the
/// same everywhere. Module names double as `@class/<module>` targets,
/// exactly as `ws_module()` establishes for workspace pills (mango.rs).
///
/// Click paths point at `~/.config/ironbar/scripts/<script>` — the surviving
/// click-only shell scripts, moved there at T8 from waybar's own
/// `config.jsonc` convention (now deleted, waybar with it). T3 keeps every
/// click-driven menu as shell (IRONBAR.md goal 4); only the polling/event
/// code moved into the daemon. `nm-connection-editor` is a real installed
/// binary, not a repo script, so it is referenced bare.
///
/// T19: `wifi`/`eth`/`netsec` gain hover detail popups (net.rs's
/// `refresh_wifi_tip`/`refresh_eth_tip`/`refresh_sec_tip`, T7b) — the last
/// three pills that still carried a static `tooltip` instead. Each moves
/// from a bare `label` to a `button` (only a real `GtkButton` registers a
/// popup anchor — a bare label never does, confirmed by every other
/// popup-bearing module in this file) and drops its `tooltip` string, whose
/// gesture hint moved into `tooltip.rs`'s `HINTS` table
/// (`wifi_tip`/`eth_tip`/`sec_tip`). `on_click_left`/`on_click_right` stay —
/// both are launches (a menu script, an external editor), never a popup
/// toggle, so neither fights hover the way a leftover `toggle-popup` would.
/// `net_modules` now takes `bar_name`, same shape as `audio_modules`/
/// `cpu_modules`, to build the `hover enter|exit <bar> <widget>` commands.
fn net_modules(bar_name: &str) -> Vec<Value> {
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
            "bar": [ { "type": "button", "label": "#wifi_text" } ],
            "popup": popup("wifi_tip", bar_name),
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} wifi -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} wifi -q"),
            "on_click_left": "~/.config/ironbar/scripts/wifi-menu.sh",
            "on_click_right": "nm-connection-editor"
        }),
        json!({
            "type": "custom",
            "name": "eth",
            "class": "eth",
            "bar": [ { "type": "button", "label": "#eth_text" } ],
            "popup": popup("eth_tip", bar_name),
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} eth -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} eth -q"),
            "on_click_left": "~/.config/ironbar/scripts/eth-toggle.sh",
            "on_click_right": "nm-connection-editor"
        }),
        json!({
            "type": "custom",
            "name": "netsec",
            "class": "netsec",
            "bar": [ { "type": "button", "label": "#sec_text" } ],
            "popup": popup("sec_tip", bar_name),
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} netsec -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} netsec -q"),
            "on_click_left": "~/.config/ironbar/scripts/net.sh --sec-click",
            "on_click_right": "~/.config/ironbar/scripts/net.sh --sec-edit"
        }),
    ]
}

/// CPU/memory pills: T6a (see cpu.rs/memory.rs), popups added at T7a. Click
/// migrates left -> `toggle-popup` / middle -> btop, same pattern
/// `power_modules`/`audio_modules`/`docker_module` already established
/// (IRONBAR.md T7a decision D7).
///
/// **Corrected live, after the first D1 design (an embedded
/// `{{interval:cmd}}` poke widget inside the popup) shipped and was measured
/// against the real running daemon: it is NOT gated by popup visibility.**
/// `mango-bard stats`' `cpu_detail_builds`/`mem_detail_builds` climbed
/// continuously from daemon startup with no popup ever opened — 2x the
/// configured interval's rate, matching 2 real monitors each running their
/// own copy of the embedded script. Ironbar's own docs (`docs/Scripts.md`,
/// fetched from the upstream repo) confirm why: a `{{mode:interval:script}}`
/// embed only ever runs in `poll` or `watch` mode, both interval-driven with
/// no visibility gate anywhere in the spec — S2's "closed popup executes
/// nothing" finding does not generalize to this shape, and cannot be relied
/// on again.
///
/// Fix: no poke widget at all. `on_click_left` runs the refresh *and* the
/// popup toggle as one `sh -c` command — ironbar's docs confirm every script
/// input, including `on_click_*`, is "passed to `sh -c`", so `;` sequences
/// two real shell commands in one string (the S2 "on_click can't be an
/// array" limitation was about the *config format*, a JSON array, never
/// about shell syntax within one string). A real click is a genuine one-off
/// user event, not a poll — refresh completes (a sub-millisecond IPC round
/// trip, S4) before the popup opens, so it always shows fresh content, and
/// nothing runs at any other time. `-q` keeps the refresh's own `ok` reply
/// out of ironbar's log; it has no visible label to pollute now that there
/// is no poke widget.
///
/// T-hover: `on_click_left`'s refresh-then-toggle-popup is gone —
/// `on_mouse_enter`/`on_mouse_exit` open/close the popup instead, via
/// `main.rs`'s debounced hover state machine (`hover enter`/`hover exit`),
/// which itself calls the `cpu-detail`/`mem-detail` refresh before opening.
/// A leftover click toggle would fight hover's own open/close. `btop`
/// (T15: moved from middle- to left-click) launches an external process
/// rather than sending `toggle-popup`, so it does not fight hover either.
fn cpu_modules(bar_name: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "custom",
            "name": "cpu",
            "class": "cpu",
            "bar": [ { "type": "button", "label": "#cpu_text" } ],
            "popup": popup("cpu_tip", bar_name),
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} cpu -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} cpu -q"),
            // T15: left-click, not middle — middle-click retired bar-wide.
            "on_click_left": "kitty --class mango-monitor -e btop"
        }),
        json!({
            "type": "custom",
            "name": "memory",
            "class": "memory",
            "bar": [ { "type": "button", "label": "#mem_text" } ],
            "popup": popup("mem_tip", bar_name),
            "on_mouse_enter": format!("mango-bard hover enter {bar_name} memory -q"),
            "on_mouse_exit": format!("mango-bard hover exit {bar_name} memory -q"),
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
///
/// T-hover: `on_click_left`'s plain `toggle-popup` is gone — hover opens/
/// closes it instead (main.rs's debounced state machine marks `power_due`
/// before opening, since power.rs already stays fresh off its own udev event
/// stream — no lazy detail build needed here, unlike cpu/memory). Powermode
/// (T15: moved from middle- to left-click) and right (powertop) are
/// untouched otherwise.
fn power_modules(bar_name: &str) -> Vec<Value> {
    vec![json!({
        "type": "custom",
        "name": "battery",
        "class": "battery",
        "bar": [ { "type": "button", "label": "#bat_text" } ],
        "popup": popup("bat_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} battery -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} battery -q"),
        "on_click_left": "~/.config/mango/scripts/powermode.sh toggle",
        "on_click_right": "kitty --class mango-monitor -e sudo -n powertop"
    })]
}

/// Docker pill: T6b (see docker.rs). Bar-global, one Docker daemon, not one
/// per monitor. Click layout (IRONBAR.md T6b decision D4) follows T4/T5:
/// left opens the popup (the container list that used to live under
/// docker-menu.sh's own summary), right keeps waybar's own left-click
/// (config.jsonc:60, the rofi quick-actions menu) — docker-menu.sh itself is
/// untouched, only the gesture that reaches it moves.
///
/// T-hover: `on_click_left`'s plain `toggle-popup` is gone — hover opens/
/// closes it instead (marks `docker_due`, same "already stays fresh off its
/// own event stream" reasoning as `power_modules`). Right-click
/// (docker-menu.sh) is untouched.
fn docker_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "docker",
        "class": "docker",
        "bar": [ { "type": "button", "label": "#docker_text" } ],
        "popup": popup("docker_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} docker -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} docker -q"),
        "on_click_right": "~/.config/ironbar/scripts/docker-menu.sh"
    })
}

/// Claude usage pill: T6c (see claude.rs). Bar-global, one account, not one
/// per monitor — same shape as `docker_module()`. Click follows T7a's
/// refresh-then-toggle-popup pattern before this stage; right click keeps
/// waybar's own `on-click` (config.jsonc:261, opening the usage settings
/// page).
///
/// **Correction, T-hover follow-up:** the plan's own scoped removal list
/// (cpu/memory/docker/battery/volume/bluetooth) missed that `claudebar` has
/// the exact same shape — a plain detail popup with no mutating click, so
/// nothing distinguishes it from the six that did lose their toggle. Leaving
/// `on_click_left`'s toggle in place caused a real bug, not just a
/// theoretical overlap: `toggle_popup` is also what `ipc.rs::show_popup`
/// sends now (`ironbar`'s own `show_popup` command rejects every `custom`
/// module — see `ipc.rs`'s doc comment), so a click arriving after hover
/// already opened the popup would close it again. Fixed the same way as
/// cpu/memory: `on_click_left` dropped entirely, `on_mouse_enter`/
/// `on_mouse_exit` are the only way this popup opens now.
fn claudebar_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "claudebar",
        "class": "claudebar",
        "bar": [ { "type": "button", "label": "#claude_text" } ],
        "popup": popup("claude_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} claudebar -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} claudebar -q"),
        "on_click_right": "xdg-open https://claude.ai/settings/usage"
    })
}

/// Hotspot pill: T6b (see hotspot.rs). Bar-global, like `docker_module()`.
///
/// T15: middle-click retired bar-wide, and hotspot moves onto the same
/// hover-opens-the-popup pattern as every other converted pill (T-hover) —
/// it was the last module still opening its popup from a click. Left keeps
/// waybar's own menu (config.jsonc:358); toggle takes the gesture middle
/// used to hold, moved to right since that is now free. The static
/// `tooltip` is dropped for the same reason T13 dropped it everywhere
/// else — it fires on the same hover as the popup and
/// `tooltip_and_hover_are_mutually_exclusive` forbids both; its gesture
/// hint moved into `tooltip.rs`'s `HINTS` table (`hotspot_tip`).
fn hotspot_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "hotspot",
        "class": "hotspot",
        "show_if": "#hotspot_show",
        "bar": [ { "type": "button", "label": "#hotspot_text" } ],
        "popup": popup("hotspot_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} hotspot -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} hotspot -q"),
        "on_click_left": "~/.config/ironbar/scripts/hotspot.sh --menu",
        "on_click_right": "~/.config/ironbar/scripts/hotspot.sh --toggle"
    })
}

/// Remote-access pill: wayvnc + kdeconnectd, one button (see remote.rs).
/// Unlike `hotspot_module` above, no `show_if` — a toggle that hides itself
/// once off has no way to be clicked back on, so this pill stays visible
/// and carries its on/off/partial state entirely through `@class/remote`
/// (style.css's `.remote.active`/`.remote.partial`).
fn remote_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "remote",
        "class": "remote",
        "bar": [ { "type": "button", "label": "#remote_text" } ],
        "popup": popup("remote_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} remote -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} remote -q"),
        "on_click_left": "~/.config/ironbar/scripts/remote.sh --toggle"
    })
}

/// Native `music` module, replacing waybar's `mpris` built-in
/// (config.jsonc:62-69). `playerctld` is waybar's own MPRIS proxy choice;
/// ironbar's `music` module talks to MPRIS directly, so no `player_type`
/// override is needed — the default already covers this.
///
/// T8c: `truncate` added — config.jsonc:66's `dynamic-len: 30` had no
/// ironbar counterpart, so a long track title could grow this module
/// without bound and push `power` off the right edge of the screen (one of
/// three unbounded-width causes found this stage; see `window_module()`
/// and `build()`'s `"height"` for the other two).
fn music_module() -> Value {
    json!({
        "type": "music",
        "name": "music",
        "class": "music",
        "truncate": { "mode": "end", "max_length": 30 },
        "tooltip": "Now playing — click to play/pause",
        "on_click_left": "playerctl play-pause"
    })
}

/// Native `bluetooth` module, replacing waybar's `bluetooth` built-in
/// (config.jsonc:374-381).
///
/// T8c: `format` reduced to the bare icon on every state (waybar's own
/// `format`/`format-connected`/`format-disabled` at config.jsonc:375-377
/// are also icon-only — the verbose "RAPOO BT4.0 Mouse • 85%" this module
/// printed by default was never parity, it was ironbar's own richer
/// built-in default). Codepoints are U+E1A7/E1A8/E1A9 in the schema's
/// default (`FormatConfig`'s own doc, Material Symbols
/// enabled/bluetooth/disabled) but those fail the same GTK4 variable-font
/// bug T8b found — U+F293 (a Nerd Font bluetooth glyph, live-confirmed
/// rendering correctly) is used for every state instead, since the pill's
/// only job here is "bluetooth is a thing you can click", not
/// distinguishing on/off at the glyph level (waybar didn't either — same
/// glyph in all three of its states).
///
/// T19: U+F293 (Font Awesome bluetooth, a filled blob) -> four distinct
/// `nf-md` bluetooth glyphs, one per state — part of the one-icon-family
/// sweep (IRONBAR.md T19): every remaining Font Awesome glyph on the bar
/// sat smaller and higher than its Material Design neighbours. This also
/// regains the per-state resolution T8c's "icon-only, same glyph every
/// state" reduction gave up — `enabled`/`disabled`/`connected`/
/// `connected_battery` now read apart at a glance, matching the schema's
/// own default four-way `FormatConfig` shape more closely than a single
/// glyph ever did. Checked with `pango-view` before wiring in.
///
/// T9: `on_click_left` is back, set to `toggle-popup` — see the finding
/// below for why. `blueman-manager` stays on `on_click_right` so the fuller
/// GUI is still one click away.
///
/// **T9 finding, live-confirmed:** ironbar's native `bluetooth` popup does
/// not respect `popup_autohide` (set bar-wide in `build()`, and confirmed
/// working for every `custom`-type popup on this bar). Live probe: opened
/// the bluetooth popup, clicked empty desktop via `ydotool`,
/// `ironbar bar <bar> get-popup-visible` stayed `true` and a scoped
/// screenshot was pixel-identical before/after. `--print-schema` confirms
/// `PopupConfig` has no autohide override — no config-side fix exists, same
/// category as T8b's cursor-property dead end.
///
/// **T-hover: this is what actually fixes the T9 gap above, not a
/// workaround for it.** `on_click_left`'s `toggle-popup` is gone —
/// `on_mouse_exit` calls `hide_popup` unconditionally (main.rs's hover state
/// machine), and `hide_popup` closes whatever is open regardless of
/// `popup_autohide`. The popup now closes the instant the cursor leaves,
/// every time, with no dependence on the GTK-level bug T9 found. No refresh
/// is dispatched on hover-open (`hover_refresh_topic` returns `None` for
/// `bluetooth`) — this is a native module with no daemon-tracked ironvar, so
/// there is nothing here for `mango-bard` to refresh. `blueman-manager`
/// stays on `on_click_right`, untouched.
fn bluetooth_module(bar_name: &str) -> Value {
    json!({
        "type": "bluetooth",
        "name": "bluetooth",
        "class": "bluetooth",
        "format": {
            "enabled": "\u{f00af}",
            "disabled": "\u{f00b0}",
            "connected": "\u{f00b1}",
            "connected_battery": "\u{f00b1}",
            "not_found": ""
        },
        // T13: static `tooltip` deleted, same as every other hover-eligible
        // module — but this is a native `bluetooth` module (ironbar's own
        // device-list popup, not a daemon-owned tip ironvar), so unlike the
        // `custom` modules there is nowhere to fold its "right-click for
        // full manager" hint into. Known gap — still documented in
        // IRONBAR.md/README.md, just not on-screen.
        //
        // T-popup-hold: `on_mouse_exit` dropped outright, not converted to
        // `hover exit` — this is the one popup this repo builds no `popup:`
        // widgets for at all (ironbar's own native device list), so there
        // is no content box to wire `hover hold`/`release` onto the way
        // `genconfig.rs::popup()` does for every `custom` module. Reported
        // symptom: the pointer crossing `popup_gap` into the device list to
        // click a device closed it first, every time. Without
        // `on_mouse_exit`, the popup instead stays open until the pointer
        // hovers a different pill (`hover_enter`'s own "different widget
        // open" branch still hides it unconditionally) or the user clicks
        // away (`popup_autohide: true`, set daemon-wide in `build()`).
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} bluetooth -q"),
        "on_click_right": "blueman-manager"
    })
}

/// Session-menu button — config.jsonc:275-279, static, no daemon var. Last
/// module of the last row, same as waybar's own placement.
fn power_module() -> Value {
    json!({
        "type": "custom",
        "name": "power",
        "class": "power",
        "bar": [ { "type": "label", "label": "\u{f0425}" } ],
        "tooltip": "Session menu",
        "on_click_left": "~/.config/mango/scripts/powermenu.sh"
    })
}

/// Darkmode pill: T6b (see darkmode.rs). Bar-global. No popup (nothing to
/// show beyond the icon itself — waybar's own tooltip was a fixed string
/// too, config.jsonc:224); a static `tooltip` string is supported (ironbar
/// schema: `tooltip` is `string|null`, only *dynamic* strings need
/// `{{script}}` and aren't supported — a fixed string needs neither).
fn darkmode_module() -> Value {
    json!({
        "type": "custom",
        "name": "darkmode",
        "class": "darkmode",
        "bar": [ { "type": "label", "label": "#dark_icon" } ],
        "tooltip": "Toggle dark/light",
        "on_click_left": "~/.config/ironbar/scripts/darkmode.sh --toggle"
    })
}

/// Color picker button — config.jsonc:216-220, static, no daemon var.
/// T8b: U+E3B8 (Material Symbols "colorize") -> U+F1FB (eyedropper, Nerd
/// Font Font Awesome) — see spark_module()'s doc comment for why.
/// T19: U+F1FB -> U+F020B (md-eyedropper_variant) — one-glyph-family sweep,
/// see IRONBAR.md's T19 entry: every remaining Font Awesome glyph on the bar
/// is smaller and sits higher than its Material Design neighbours, since the
/// two families are patched from different source fonts with different
/// vertical scaling. Checked with `pango-view --font="JetBrainsMono Nerd
/// Font Propo"` before wiring in, same method as T18.
fn colorpicker_module() -> Value {
    json!({
        "type": "custom",
        "name": "colorpicker",
        "class": "colorpicker",
        "bar": [ { "type": "label", "label": "\u{f020b}" } ],
        "tooltip": "Pick a color",
        "on_click_left": "hyprpicker -a"
    })
}

/// Screenshot-region button — config.jsonc:227-231, static, no daemon var.
/// T8b: U+F7D2 (Material Symbols "screenshot_region") -> U+F125 (crop, Nerd
/// Font Font Awesome) — see spark_module()'s doc comment for why.
/// T19: U+F125 -> U+F0E5A (md-monitor_screenshot) — a monitor outline with a
/// dashed selection inside it, closer to the original Material Symbols
/// "screenshot_region" than a bare crop glyph, and part of the same
/// one-icon-family sweep as colorpicker_module() above.
///
/// T20: both clicks routed through screenshot.sh so each mode saves a
/// file, copies it, and notifies the same way — the old copy-only
/// `grim | wl-copy` inline pipeline is gone. Shift-modified clicks were
/// the first choice and are impossible: `ironbar --print-schema` types
/// every `on_click_*` as a plain `ScriptInput` with no modifier variants;
/// middle-click is out too (no middle button on this machine's mouse, per
/// user). Two buttons carry the two pointer-driven modes — region and
/// window, both of which need the pointer next anyway — and the two
/// pointer-free modes (active monitor, every output) stay on their
/// config.conf keys, named in the tooltip so the split is discoverable.
fn snip_module() -> Value {
    json!({
        "type": "custom",
        "name": "snip",
        "class": "snip",
        "bar": [ { "type": "label", "label": "\u{f0e5a}" } ],
        "tooltip": "Screenshot — left: region, right: window (monitor: Alt+S, all: Print)",
        "on_click_left": "~/.config/mango/scripts/screenshot.sh region",
        "on_click_right": "~/.config/mango/scripts/screenshot.sh window"
    })
}

/// Keep-awake pill — was ironbar's native `inhibit` module (`"type":
/// "inhibit"`, replacing waybar's `idle_inhibitor` built-in,
/// config.jsonc:236-241), now a `custom` module driven by keepawake.rs, same
/// shape as `remote_module()` above. The native module's
/// `gtk_application_inhibit()` call needs `org.freedesktop.portal.Inhibit`,
/// a portal interface this session has no backend for — confirmed live,
/// three times, in `~/.local/share/ironbar/ironbar.*.log`: `Cannot get
/// portal org.freedesktop.portal.Inhibit version: ... No such interface`.
/// The toggle changed the coffee glyph and inhibited nothing; see
/// keepawake.rs's own doc comment for the full root cause and the
/// `systemd-inhibit` replacement.
///
/// This also closes the gap the native module's own old doc comment used
/// to flag here: no documented on/off CSS class to hang an accent-filled
/// state on. A `custom` module carries its own `@class/inhibit` ironvar
/// (keepawake.rs), so `style.css`'s `.inhibit.active` rule now has
/// something to select.
///
/// Glyphs carried over unchanged from the native module's own
/// `format_on`/`format_off`: `nf-md-coffee` (U+F0176, filled cup on a
/// saucer) and `nf-md-coffee_outline` (U+F06CA, the same cup as an
/// outline) — both size to the same 832/1000 em advance in the Propo font,
/// so the pill doesn't change width when it toggles. Full glyph-choice
/// history (Font Awesome/Codicon mismatch, symbol-cmap fallback, PUA
/// plane) is on keepawake.rs's `IC_ON`/`IC_OFF` constants now.
fn inhibit_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "inhibit",
        "class": "inhibit",
        "bar": [ { "type": "button", "label": "#inhibit_text" } ],
        "popup": popup("inhibit_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} inhibit -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} inhibit -q"),
        "on_click_left": "~/.config/ironbar/scripts/keepawake.sh --toggle"
    })
}

/// Nine numbered pills plus one overview pill for `mon` — and nothing else.
/// `build()` puts exactly this in the bar's `center` slot, so the pill group
/// (a fixed 324px — see `.ws-overview`'s own width comment in style.css) is
/// the only thing GTK centres there; clock/pomo/colorpicker/darkmode/snip/
/// inhibit used to trail this same array (T8a), which meant `center` really
/// held 15 variable-width modules, not the 9 pills a viewer reads as "the
/// centre" — any of those other 6 changing width (a longer pomodoro label,
/// a class toggle) shifted the pills sideways. They moved to
/// `rightcenter_modules()`, appended to `end` instead (see that function's
/// own doc comment).
///
/// Each pill is a `custom` module (not a widget) because `style
/// add-class`/`remove-class` match module names, and `popup` exists only on
/// `custom` modules.
///
/// `on_click_left`/`on_click_right` are module-level `ScriptInput`, which
/// runs its string as a plain shell command — unlike a *widget*-level
/// `on_click`, which is overloaded between built-in actions
/// (`popup:toggle`) and a shell command disambiguated by a leading `!`.
/// Verified live (T2 probe V1): no `!` prefix here.
///
/// T14: `justify: "center"` on the bar button — `mango.rs::tag_label`
/// renders a two-line label (tag number, then a narrower dot row at 38%
/// size), and `ButtonWidget.justify` defaults to `left`, which put the
/// number flush against the pill's left edge instead of centred over its
/// own dot row. Every other pill on the bar is single-line, where
/// `justify` is a no-op, so only this one needs it.
///
/// T18: T14's placement was dead — ironbar's `ButtonWidget` parses
/// `justify` (it is a valid config field) but never applies it to the
/// label it builds (`src/modules/custom/button.rs`: `Label::new(None)`,
/// no `set_justify` call anywhere in `into_widget`). Only `LabelWidget`
/// calls `label.set_justify(...)` (`src/modules/custom/label.rs`). Fixed
/// by nesting a real `LabelWidget` inside the button's `widgets`, the same
/// shape `window_module()` already uses for `truncate` (a field that
/// likewise only exists on `LabelWidget`) — `justify` now reaches the
/// label that actually needs it.
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
                "bar": [ { "type": "button", "widgets": [
                    { "type": "label", "justify": "center", "label": format!("#{}", var_lbl(slug, n)) }
                ] } ],
                "popup": popup(&var_tip(slug, n), &bar_name),
                "on_click_left": format!("mmsg dispatch view,{n},0"),
                // T-hover: hover opens/closes the popup; right-click's own
                // toggle-popup stays too, as a harmless manual fallback —
                // unlike cpu/memory/docker/battery/volume/bluetooth, this
                // gesture isn't in tension with hover (it's a different
                // click, not the redundant one hover replaced). No refresh
                // is dispatched on hover-open here: mango.apply() already
                // keeps every tag's tip live off its own mmsg event stream,
                // so there is nothing stale to lazily rebuild.
                "on_mouse_enter": format!("mango-bard hover enter {bar_name} {module} -q"),
                "on_mouse_exit": format!("mango-bard hover exit {bar_name} {module} -q"),
                "on_click_right": format!("ironbar bar {bar_name} toggle-popup {module}"),
                "on_scroll_up": "mmsg dispatch viewtoleft,0",
                "on_scroll_down": "mmsg dispatch viewtoright,0",
                // T-next (item 5): default `show_if` transition is
                // `slide_start` — a left-to-right slide, which read as the
                // pill group shuffling sideways rather than a tag simply
                // appearing/disappearing. Ironbar has no slide-from-top
                // option (`--print-schema`: slide_start/slide_end/
                // crossfade/none only), so a fade is the closest match.
                "transition_type": "crossfade"
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
        "bar": [ { "type": "label", "label": overview_label() } ],
        "tooltip": "Overview — click to exit",
        "on_click_left": "mmsg dispatch toggleoverview,",
        "transition_type": "crossfade"
    }));
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

    /// Every `popup()` call returns exactly one top-level widget: the
    /// vertical box T-popup-vert wraps everything in. Pulls its nested
    /// `widgets` array back out so the rest of these tests can assert on
    /// the actual title/sep/body/sep/hint sequence.
    fn popup_inner(tip_key: &str) -> Vec<Value> {
        let outer = popup(tip_key, "bar-eDP-1");
        let outer = outer.as_array().unwrap();
        assert_eq!(outer.len(), 1, "{tip_key}: popup must have one top widget");
        assert_eq!(outer[0]["type"], json!("box"));
        assert_eq!(outer[0]["orientation"], json!("vertical"));
        outer[0]["widgets"].as_array().unwrap().clone()
    }

    #[test]
    fn popup_box_holds_and_releases_on_its_own_bar() {
        // T-popup-hold: the outer box must carry hover hold/release naming
        // its own bar, not a hardcoded one — see popup()'s own doc comment.
        let outer = popup("cpu_tip", "bar-DP-1");
        let outer = outer.as_array().unwrap();
        assert_eq!(
            outer[0]["on_mouse_enter"],
            json!("mango-bard hover hold bar-DP-1 -q")
        );
        assert_eq!(
            outer[0]["on_mouse_exit"],
            json!("mango-bard hover release bar-DP-1 -q")
        );
    }

    #[test]
    fn popup_adds_a_real_separator_and_hint_for_every_hints_key() {
        // Pins the shape `popup()` builds for a keyed tip, so tooltip::HINTS
        // and genconfig.rs's popup widgets can't drift apart.
        for (key, hint) in crate::tooltip::HINTS {
            let widgets = popup_inner(key);
            let hint_at = widgets.len() - 2;
            assert_eq!(widgets[hint_at]["type"], json!("box"));
            assert_eq!(widgets[hint_at]["class"], json!("popup-sep"));
            assert_eq!(widgets[hint_at + 1]["type"], json!("label"));
            assert_eq!(widgets[hint_at + 1]["class"], json!("popup-hint"));
            assert_eq!(widgets[hint_at + 1]["label"], json!(*hint));
        }
    }

    #[test]
    fn popup_adds_a_title_and_separator_for_every_titled_key() {
        // Mirrors the HINTS test above, for tooltip::TITLED.
        for key in crate::tooltip::TITLED {
            let widgets = popup_inner(key);
            assert_eq!(widgets[0]["type"], json!("label"));
            assert_eq!(widgets[0]["class"], json!("popup-title"));
            assert_eq!(widgets[0]["label"], json!(format!("#{key}_title")));
            assert_eq!(widgets[1]["type"], json!("box"));
            assert_eq!(widgets[1]["class"], json!("popup-sep"));
        }
    }

    #[test]
    fn popup_body_label_is_always_present() {
        for key in ["clk_tip", "win_tip", "cpu_tip", "hotspot_tip"] {
            let widgets = popup_inner(key);
            let has_body = widgets
                .iter()
                .any(|w| w["type"] == json!("label") && w["label"] == json!(format!("#{key}")));
            assert!(has_body, "{key}: no body label found in {widgets:?}");
        }
    }

    #[test]
    fn popup_stays_title_and_body_only_with_no_hints_entry() {
        // clk_tip is TITLED but not in HINTS: title, sep, body — no more.
        let widgets = popup_inner("clk_tip");
        assert_eq!(widgets.len(), 3);
    }

    #[test]
    fn popup_is_a_single_body_label_with_neither_title_nor_hints() {
        // Every ws_<slug>_<n>_tip: no title, no hint, just the body.
        let widgets = popup_inner("ws_eDP-1_1_tip");
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0]["type"], json!("label"));
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
        // T8d: center[0] is no longer a workspace pill — leftcenter_modules()
        // now precedes the workspace pills, so find the first tag pill by
        // its known name instead of assuming an index.
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let ws1 = center
            .iter()
            .find(|m| m["name"] == ws_module("eDP-1", 1))
            .unwrap();
        let click = ws1["on_click_right"].as_str().unwrap();
        assert!(click.contains("bar-eDP-1"));
        assert!(
            !click.starts_with('!'),
            "module-level ScriptInput needs no ! prefix"
        );
    }

    #[test]
    fn volume_hover_targets_its_own_bar_name_and_drops_the_click_toggle() {
        // T-hover: volume's plain toggle-popup on_click_left is gone —
        // on_mouse_enter/on_mouse_exit target the bar instead.
        // T15: on_click_left is not absent any more — it's the tabbed mixer
        // shortcut that used to live on middle-click, freed by the toggle
        // removal above.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let volume = end.iter().find(|m| m["name"] == "volume").unwrap();
        assert_eq!(volume["on_click_left"], json!("pavucontrol-qt -t 5"));
        assert_eq!(volume["on_click_right"], json!("pavucontrol-qt"));
        assert!(volume.get("on_click_middle").is_none());
        assert!(volume["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert!(volume["on_mouse_exit"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));

        // The fallback bar (no monitor listed) must also get a name to
        // target, per T0 spike S3 — an unlisted output still gets this bar.
        let fallback = build(&[]);
        assert_eq!(fallback["name"], json!("bar-default"));
        let fb_end = fallback["end"].as_array().unwrap();
        let fb_volume = fb_end.iter().find(|m| m["name"] == "volume").unwrap();
        assert!(fb_volume["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    #[test]
    fn battery_hover_targets_its_own_bar_name_and_drops_the_click_toggle() {
        // T8d: battery moved from `end` to `center` (leftcenter_modules).
        // T-hover: see volume_hover_targets_its_own_bar_name_and_drops_the_
        // click_toggle — same shape.
        // T15: on_click_left is powermode toggle now, moved from middle;
        // right stays untouched.
        // T-next (item 3): `center` renamed to `start` — see
        // `leftcenter_modules()`'s own doc comment.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let battery = start.iter().find(|m| m["name"] == "battery").unwrap();
        assert!(battery["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert!(battery["on_mouse_exit"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert_eq!(
            battery["on_click_left"],
            json!("~/.config/mango/scripts/powermode.sh toggle")
        );
        assert!(battery.get("on_click_middle").is_none());
        assert!(battery["on_click_right"].is_string());

        let fallback = build(&[]);
        let fb_start = fallback["start"].as_array().unwrap();
        let fb_battery = fb_start.iter().find(|m| m["name"] == "battery").unwrap();
        assert!(fb_battery["on_mouse_enter"]
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
    fn cpu_memory_docker_battery_claudebar_music_sit_at_the_back_of_start() {
        // T8d: waybar's own `group/leftcenter` order (config.jsonc:26-29).
        // T-next (item 3): moved again, from the front of `center` to the
        // back of `start` (after spark/win) — see `leftcenter_modules()`'s
        // own doc comment for why `center` had to lose this group.
        // `claudebar` (T6c) sits in the same slot waybar's own module list
        // gives it, between battery and mpris/music.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let names: Vec<&str> = start.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let win = names.iter().position(|n| *n == "win").unwrap();
        let cpu = names.iter().position(|n| *n == "cpu").unwrap();
        let memory = names.iter().position(|n| *n == "memory").unwrap();
        let docker = names.iter().position(|n| *n == "docker").unwrap();
        let battery = names.iter().position(|n| *n == "battery").unwrap();
        let claudebar = names.iter().position(|n| *n == "claudebar").unwrap();
        let music = names.iter().position(|n| *n == "music").unwrap();
        assert!(win < cpu, "leftcenter must follow spark/win in start");
        assert!(
            cpu < memory
                && memory < docker
                && docker < battery
                && battery < claudebar
                && claudebar < music
        );
    }

    #[test]
    fn end_no_longer_carries_the_leftcenter_modules() {
        // T8d regression guard: these five must not still be in `end`.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        for name in ["cpu", "memory", "docker", "battery", "claudebar", "music"] {
            assert!(
                !end.iter().any(|m| m["name"] == name),
                "{name} must have moved to start, not stayed in end"
            );
        }
    }

    #[test]
    fn hotspot_sits_after_net_pills_in_end() {
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let netsec = names.iter().position(|n| *n == "netsec").unwrap();
        let hotspot = names.iter().position(|n| *n == "hotspot").unwrap();
        assert!(netsec < hotspot);
    }

    #[test]
    fn docker_hover_targets_its_own_bar_name_and_drops_the_click_toggle() {
        // T8d: docker moved from `end` to `center` (leftcenter_modules).
        // T-next (item 3): `center` renamed to `start` — see
        // `leftcenter_modules()`'s own doc comment.
        // T-hover: plain toggle-popup on_click_left is gone — right-click
        // (docker-menu.sh) stays untouched.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let docker = start.iter().find(|m| m["name"] == "docker").unwrap();
        assert!(docker.get("on_click_left").is_none());
        assert!(docker["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert!(docker["on_mouse_exit"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert_eq!(
            docker["on_click_right"],
            json!("~/.config/ironbar/scripts/docker-menu.sh")
        );

        let fallback = build(&[]);
        let fb_start = fallback["start"].as_array().unwrap();
        let fb_docker = fb_start.iter().find(|m| m["name"] == "docker").unwrap();
        assert!(fb_docker["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    #[test]
    fn claudebar_popup_targets_its_own_bar_name() {
        // T-next (item 3): `center` renamed to `start` — see
        // `leftcenter_modules()`'s own doc comment.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let claudebar = start.iter().find(|m| m["name"] == "claudebar").unwrap();
        assert!(claudebar.get("on_click_left").is_none());
        assert!(claudebar["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert_eq!(
            claudebar["on_click_right"],
            json!("xdg-open https://claude.ai/settings/usage")
        );

        let fallback = build(&[]);
        let fb_start = fallback["start"].as_array().unwrap();
        let fb_claudebar = fb_start.iter().find(|m| m["name"] == "claudebar").unwrap();
        assert!(fb_claudebar["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    #[test]
    fn hotspot_hover_targets_its_own_bar_name() {
        // T15: hotspot's popup opens on hover now, not on right-click (that
        // gesture is `hotspot.sh --toggle` instead) — same shape every
        // other converted pill uses.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let hotspot = end.iter().find(|m| m["name"] == "hotspot").unwrap();
        assert!(hotspot["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert!(hotspot["on_mouse_exit"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert_eq!(
            hotspot["on_click_right"],
            json!("~/.config/ironbar/scripts/hotspot.sh --toggle")
        );
        assert_eq!(
            hotspot["on_click_left"],
            json!("~/.config/ironbar/scripts/hotspot.sh --menu")
        );
    }

    #[test]
    fn darkmode_appears_once_per_monitor_and_on_the_fallback_bar() {
        // T-next (item 3): darkmode is part of `rightcenter_modules()`,
        // which lives in `end` on a real per-monitor bar now (the fallback
        // bar below has no pill group to protect, so it keeps this group in
        // `center` — see `rightcenter_modules()`'s own doc comment).
        let cfg = build(&["eDP-1".to_string(), "DP-1".to_string()]);
        for mon in ["eDP-1", "DP-1"] {
            let end = cfg["monitors"][mon]["end"].as_array().unwrap();
            let count = end.iter().filter(|m| m["name"] == "darkmode").count();
            assert_eq!(count, 1, "{mon} must have exactly one darkmode module");
        }
        let fallback = build(&[]);
        let fb_center = fallback["center"].as_array().unwrap();
        assert!(fb_center.iter().any(|m| m["name"] == "darkmode"));
    }

    // ---- T7a additions

    #[test]
    fn cpu_and_memory_hover_targets_its_own_bar_name_and_drops_the_click_toggle() {
        // T8d: cpu/memory moved from `end` to `center` (leftcenter_modules).
        // T-next (item 3): `center` renamed to `start` — see
        // `leftcenter_modules()`'s own doc comment.
        // T-hover: the refresh-then-toggle-popup on_click_left is gone —
        // hover carries the bar name instead (main.rs's hover state machine
        // runs the cpu-detail/mem-detail refresh before opening).
        // T15: btop moved from middle- to left-click.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        for name in ["cpu", "memory"] {
            let module = start.iter().find(|m| m["name"] == name).unwrap();
            assert!(module["on_mouse_enter"]
                .as_str()
                .unwrap()
                .contains("bar-eDP-1"));
            assert!(module["on_mouse_exit"]
                .as_str()
                .unwrap()
                .contains("bar-eDP-1"));
            assert_eq!(
                module["on_click_left"],
                json!("kitty --class mango-monitor -e btop")
            );
            assert!(module.get("on_click_middle").is_none());
            // T-popup-vert: popup is one outer vertical box now (see
            // popup()'s own doc comment); no poke widget inside it either —
            // title, sep, content label, sep, hint label, no widget doing a
            // refresh-on-open the S2 finding ruled out.
            let popup = module["popup"].as_array().unwrap();
            assert_eq!(popup.len(), 1);
            assert_eq!(popup[0]["widgets"].as_array().unwrap().len(), 5);
        }

        let fallback = build(&[]);
        let fb_start = fallback["start"].as_array().unwrap();
        let fb_cpu = fb_start.iter().find(|m| m["name"] == "cpu").unwrap();
        assert!(fb_cpu["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    // ---- T-next additions: clock/date popups (T7c-rest).

    #[test]
    fn clock_and_date_hover_opens_the_popup_with_no_click() {
        // Correction, T-hover follow-up: clock/date used to carry a
        // refresh-then-toggle-popup on_click_left, same shape as
        // cpu/memory — dropped for the same reason those six lost theirs.
        // `hover_refresh_topic` (main.rs) still maps `clock`/`date` to
        // `clock-detail`/`date-detail`, so the lazy popup content stays
        // built on hover, just no longer on click too.
        //
        // T15: `date` gains an `on_click_left` again — the calendar-app
        // launch that used to sit on middle-click. It never sends
        // `toggle-popup`, so it does not reopen the collision this test
        // guards against; only `clock` (no left-click action of its own) is
        // checked for the absence.
        // T-next (item 3): `clock`/`date` are part of `rightcenter_modules()`,
        // which lives in `end` on a real per-monitor bar now — see that
        // function's own doc comment.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let clock = end.iter().find(|m| m["name"] == "clock").unwrap();
        assert!(
            clock.get("on_click_left").is_none(),
            "clock must not carry on_click_left"
        );
        // Both clk_tip and date_tip are TITLED, so both carry a title+sep
        // pair. clk_tip has no HINTS entry (its popup demonstrates world
        // times, nothing to hint at), so it stops at title+sep+body; date_tip
        // does ("click: open calendar app"), so it gains the sep+hint pair
        // `popup()` adds for any keyed tip on top of that.
        let expected_len = [("clock", 3), ("date", 5)];
        for (name, len) in expected_len {
            let module = end.iter().find(|m| m["name"] == name).unwrap();
            let enter = module["on_mouse_enter"].as_str().unwrap();
            assert!(enter.contains("bar-eDP-1"));
            assert!(enter.contains(&format!("hover enter bar-eDP-1 {name}")));
            let popup = module["popup"].as_array().unwrap();
            assert_eq!(popup.len(), 1);
            assert_eq!(popup[0]["widgets"].as_array().unwrap().len(), len);
        }

        let fallback = build(&[]);
        let fb_center = fallback["center"].as_array().unwrap();
        let fb_clock = fb_center.iter().find(|m| m["name"] == "clock").unwrap();
        assert!(fb_clock["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    #[test]
    fn date_left_click_opens_the_calendar_app() {
        // T15: moved from middle- to left-click — middle-click retired
        // bar-wide.
        // T-next (item 3): `date` is part of `rightcenter_modules()`, which
        // lives in `end` on a real per-monitor bar now.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let date = end.iter().find(|m| m["name"] == "date").unwrap();
        assert_eq!(
            date["on_click_left"],
            json!("~/.config/ironbar/scripts/clock.sh --calendar")
        );
        assert!(date.get("on_click_middle").is_none());
    }

    // ---- T8a additions: tray, bluetooth, music, inhibit, static buttons.

    #[test]
    fn tray_leads_its_own_group_in_end() {
        // T9: moved back to `end`, matching waybar's own
        // `modules-right = [tray, group/indicators]` order — see
        // `end_modules()`'s doc comment for why the box-nesting risk that
        // originally justified `start` (T8a Phase 2) never applied to tray
        // (it has no popup).
        //
        // T-next (item 3): tray no longer leads `end` as a whole — build()
        // prepends `rightcenter_modules()` (clock/pomo/colorpicker/
        // darkmode/snip/inhibit) ahead of `end_modules()`'s own return
        // value, so those lead the row instead. Tray still leads its own
        // sub-group (volume/net/hotspot/bluetooth/power), which is what
        // `end_modules()` itself controls.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        assert!(!start.iter().any(|m| m["name"] == "tray"));
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let tray = names.iter().position(|n| *n == "tray").unwrap();
        let volume = names.iter().position(|n| *n == "volume").unwrap();
        assert!(tray < volume, "tray must lead its own group, ahead of volume");
    }

    #[test]
    fn spark_precedes_window_in_start() {
        // T9: tray no longer lives here — see tray_leads_its_own_group_in_end.
        // T-next (item 3): leftcenter_modules() now trails spark/win in
        // start — see that function's own doc comment — so `start` no
        // longer stops at exactly these two.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let names: Vec<&str> = start.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(&names[..2], &["spark", "win"]);

        let fallback = build(&[]);
        let fb_start = fallback["start"].as_array().unwrap();
        assert!(fb_start.iter().any(|m| m["name"] == "spark"));
    }

    #[test]
    fn bluetooth_and_power_land_in_end() {
        // T8d: music moved to `center` (leftcenter_modules) alongside
        // battery, matching waybar's own layout — only bluetooth/power (plus
        // hotspot, checked separately) stay in `end` from this trio.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let hotspot = names.iter().position(|n| *n == "hotspot").unwrap();
        let bluetooth = names.iter().position(|n| *n == "bluetooth").unwrap();
        let power = names.iter().position(|n| *n == "power").unwrap();
        assert!(hotspot < bluetooth && bluetooth < power);
        // power is the last module of the last row, matching waybar's own
        // placement (config.jsonc:273's comment).
        assert_eq!(names.last(), Some(&"power"));
    }

    #[test]
    fn bluetooth_is_icon_only_and_hovers_its_own_popup() {
        // T8c: format reduced to the bare icon on every state (waybar
        // parity — its own format/format-connected/format-disabled are
        // also icon-only). T-hover: on_click_left's toggle-popup is gone —
        // hover-exit's unconditional hide_popup used to be what fixed the
        // T9 popup_autohide gap, so a second click was no longer needed to
        // close it.
        //
        // T-popup-hold: `on_mouse_exit` removed outright — bluetooth is a
        // native module with no `popup:` content box of ours to wire
        // `hover hold`/`release` onto, so `on_mouse_exit` closed the popup
        // the instant the pointer crossed `popup_gap` toward it, every
        // time (the reported "can never reach it" case) — see
        // `bluetooth_module()`'s own doc comment.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let bt = end.iter().find(|m| m["name"] == "bluetooth").unwrap();
        let format = bt["format"].as_object().unwrap();
        for (key, value) in format {
            let text = value.as_str().unwrap();
            assert!(
                !text.contains('{'),
                "format.{key} still has a placeholder: {text}"
            );
        }
        assert!(bt.get("on_click_left").is_none());
        let enter = bt["on_mouse_enter"].as_str().unwrap();
        assert!(enter.contains("hover enter"));
        assert!(enter.contains("bar-eDP-1"));
        assert!(enter.contains("bluetooth"));
        assert!(
            bt.get("on_mouse_exit").is_none(),
            "bluetooth must not close on hover-exit — see popup_gap comment above"
        );
        assert_eq!(bt["on_click_right"], "blueman-manager");
    }

    #[test]
    fn colorpicker_darkmode_snip_inhibit_follow_clock_date() {
        // T-next (item 3): this whole group is `rightcenter_modules()`,
        // which lives in `end` on a real per-monitor bar now.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        // T8c: clock_pill() now returns two NAMED modules ("clock", "date")
        // instead of one unnamed "pill" module — find date by name.
        let names: Vec<Option<&str>> = end.iter().map(|m| m["name"].as_str()).collect();
        let clock = names.iter().position(|n| *n == Some("clock")).unwrap();
        let date = names.iter().position(|n| *n == Some("date")).unwrap();
        let colorpicker = names
            .iter()
            .position(|n| *n == Some("colorpicker"))
            .unwrap();
        let darkmode = names.iter().position(|n| *n == Some("darkmode")).unwrap();
        let snip = names.iter().position(|n| *n == Some("snip")).unwrap();
        let inhibit = names.iter().position(|n| *n == Some("inhibit")).unwrap();
        assert!(
            clock < date
                && date < colorpicker
                && colorpicker < darkmode
                && darkmode < snip
                && snip < inhibit
        );

        let fallback = build(&[]);
        let fb_center = fallback["center"].as_array().unwrap();
        for name in ["colorpicker", "darkmode", "snip", "inhibit"] {
            assert!(
                fb_center.iter().any(|m| m["name"] == name),
                "fallback bar missing {name}"
            );
        }
    }

    #[test]
    fn inhibit_module_toggles_keepawake() {
        // The native `inhibit` module's on_click was left unset (it had no
        // working click enum on the installed binary — see genconfig.rs
        // history). The `custom` replacement fixes that: a real
        // keepawake.sh --toggle command that starts/stops
        // mango-keepawake.service, and no on_click_right (same shape as
        // remote_module(), which has none either).
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let inhibit = end.iter().find(|m| m["name"] == "inhibit").unwrap();
        assert_eq!(
            inhibit["on_click_left"],
            json!("~/.config/ironbar/scripts/keepawake.sh --toggle")
        );
        assert!(inhibit.get("on_click_right").is_none());
    }

    #[test]
    fn inhibit_module_has_a_hover_popup() {
        // Same shape as remote_module(): a real popup and both hover
        // attributes, unlike the native `inhibit` module it replaced.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let inhibit = end.iter().find(|m| m["name"] == "inhibit").unwrap();
        assert!(inhibit.get("popup").is_some());
        assert!(inhibit["on_mouse_enter"].is_string());
        assert!(inhibit["on_mouse_exit"].is_string());
    }

    #[test]
    fn native_modules_carry_their_own_class() {
        // T-next (item 3): `music` is part of `leftcenter_modules()`, which
        // lives in `start` now — see that function's own doc comment.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        for (row, name, expect_type) in [
            (start, "music", "music"),
            (end, "bluetooth", "bluetooth"),
            (end, "tray", "tray"),
        ] {
            let m = row.iter().find(|m| m["name"] == name).unwrap();
            assert_eq!(m["type"], json!(expect_type));
            assert_eq!(m["class"], json!(name));
        }
    }

    // ---- T9 additions: tray→end, edge popup_gap, tooltips, icon-only volume.

    #[test]
    fn popup_gap_is_raised_on_every_bar() {
        // T9: the only distance lever the schema exposes for "popup sits too
        // close to the button" (no per-widget offset field exists).
        let cfg = build(&["eDP-1".to_string()]);
        assert_eq!(cfg["monitors"]["eDP-1"]["popup_gap"], json!(12));
        let fallback = build(&[]);
        assert_eq!(fallback["popup_gap"], json!(12));
    }

    #[test]
    fn volume_bar_text_is_icon_only() {
        // T9: percentage moved to the popup — see audio_modules()'s doc
        // comment for why the SliderWidget alternative was rejected.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let volume = end.iter().find(|m| m["name"] == "volume").unwrap();
        assert_eq!(volume["bar"][0]["label"], json!("#vol_text"));
        // vol_text's own content (icon-only, no "%") is audio.rs's job —
        // this only asserts genconfig still points at the same ironvar.
    }

    #[test]
    fn tooltip_and_hover_are_mutually_exclusive() {
        // T13: replaces the old spot-check `every_clickable_module_has_a_
        // tooltip` — a tooltip and a hover popup fire on the same gesture
        // and compete for the same space, so tooltips were deleted from
        // every hover-eligible module (the gestures they documented moved
        // into the popup itself, see tooltip.rs's HINTS). A strict
        // superset of the old guarantee: the exhaustive mutual-exclusion
        // rule below covers every module in every row (old test only
        // spot-checked specific names), plus the same "still documented"
        // check for the modules that keep a click-only tooltip, plus an
        // explicit check that the nine converted modules really lost
        // theirs.
        let cfg = build(&["eDP-1".to_string()]);
        for row in ["start", "center", "end"] {
            for m in cfg["monitors"]["eDP-1"][row].as_array().unwrap() {
                assert!(
                    !(m["tooltip"].is_string() && m["on_mouse_enter"].is_string()),
                    "{} carries both a tooltip and on_mouse_enter",
                    m["name"]
                );
            }
        }

        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();

        let win = start.iter().find(|m| m["name"] == "win").unwrap();
        assert!(win["tooltip"].is_string());
        // T-next (item 3): `music` is part of `leftcenter_modules()`, which
        // lives in `start` now.
        let music = start.iter().find(|m| m["name"] == "music").unwrap();
        assert!(music["tooltip"].is_string(), "music missing a tooltip");
        // T19: wifi/eth/netsec moved out of this group — see below.
        let mic = end.iter().find(|m| m["name"] == "mic").unwrap();
        assert!(mic["tooltip"].is_string(), "mic missing a tooltip");

        // T-next (item 3): all five are part of `leftcenter_modules()`,
        // which lives in `start` now.
        for name in ["cpu", "memory", "docker", "battery", "claudebar"] {
            let m = start.iter().find(|m| m["name"] == name).unwrap();
            assert!(
                m["tooltip"].is_null(),
                "{name} should have lost its tooltip"
            );
        }
        // T15: hotspot joins this group — its static tooltip moved into
        // tooltip::HINTS when it gained a hover popup.
        // T19: wifi/eth/netsec join too — same reasoning (T7b popups).
        for name in ["volume", "bluetooth", "hotspot", "wifi", "eth", "netsec"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
            assert!(
                m["tooltip"].is_null(),
                "{name} should have lost its tooltip"
            );
        }
        let ws1 = center
            .iter()
            .find(|m| m["name"] == ws_module("eDP-1", 1))
            .unwrap();
        assert!(
            ws1["tooltip"].is_null(),
            "workspace pill should have lost its tooltip"
        );
    }

    // ---- T-hover additions: click -> hover conversion.

    #[test]
    fn every_hover_eligible_module_carries_both_mouse_attributes() {
        // The plan's own exhaustive "modules to convert" list: every module
        // that already had a `popup` field, except `window` (left out of
        // the plan's own list — see the stage's IRONBAR.md entry).
        // T-next (item 3): cpu/memory/docker/battery/claudebar are part of
        // `leftcenter_modules()` (now in `start`); clock/date/pomo are part
        // of `rightcenter_modules()` (now in `end` on a real per-monitor
        // bar) — see both functions' own doc comments.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        for name in ["cpu", "memory", "docker", "battery", "claudebar"] {
            let m = start.iter().find(|m| m["name"] == name).unwrap();
            assert!(
                m["on_mouse_enter"].is_string(),
                "{name} missing on_mouse_enter"
            );
            assert!(
                m["on_mouse_exit"].is_string(),
                "{name} missing on_mouse_exit"
            );
        }
        for name in ["clock", "date", "pomo"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
            assert!(
                m["on_mouse_enter"].is_string(),
                "{name} missing on_mouse_enter"
            );
            assert!(
                m["on_mouse_exit"].is_string(),
                "{name} missing on_mouse_exit"
            );
        }
        // T19: wifi/eth/netsec join volume here — see net_modules()'s own
        // doc comment. T-popup-hold: bluetooth dropped from this list — it
        // no longer carries on_mouse_exit at all (see bluetooth_module()'s
        // own doc comment), checked separately by
        // bluetooth_is_icon_only_and_hovers_its_own_popup.
        for name in ["volume", "wifi", "eth", "netsec"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
            assert!(
                m["on_mouse_enter"].is_string(),
                "{name} missing on_mouse_enter"
            );
            assert!(
                m["on_mouse_exit"].is_string(),
                "{name} missing on_mouse_exit"
            );
        }
        let bluetooth = end.iter().find(|m| m["name"] == "bluetooth").unwrap();
        assert!(
            bluetooth["on_mouse_enter"].is_string(),
            "bluetooth missing on_mouse_enter"
        );
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        for n in 1..=TAG_COUNT {
            let module = ws_module("eDP-1", n);
            let m = center.iter().find(|m| m["name"] == module).unwrap();
            assert!(
                m["on_mouse_enter"].is_string(),
                "{module} missing on_mouse_enter"
            );
            assert!(
                m["on_mouse_exit"].is_string(),
                "{module} missing on_mouse_exit"
            );
        }
    }

    #[test]
    fn non_popup_modules_carry_no_mouse_attributes() {
        // A pill with no `popup` field has nothing to show — hover there
        // must stay a no-op, so these must carry neither attribute.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let all: Vec<&Value> = start
            .iter()
            .chain(center.iter())
            .chain(end.iter())
            .collect();
        // T19: wifi/eth/netsec dropped from this list — they now carry a
        // hover popup (net_modules()'s own doc comment) and are checked by
        // every_hover_eligible_module_carries_both_mouse_attributes instead.
        // `inhibit` dropped for the same reason, this stage: it went from
        // the native `inhibit` module (no popup) to a `custom` one with a
        // hover popup (inhibit_module()'s own doc comment) — checked by
        // inhibit_module_has_a_hover_popup below instead.
        for name in [
            "spark",
            "power",
            "colorpicker",
            "snip",
            "darkmode",
            "mic",
            "net-spinner",
        ] {
            let m = all
                .iter()
                .find(|m| m["name"] == name)
                .unwrap_or_else(|| panic!("{name} not found in generated config"));
            assert!(
                m.get("on_mouse_enter").is_none(),
                "{name} must not carry on_mouse_enter"
            );
            assert!(
                m.get("on_mouse_exit").is_none(),
                "{name} must not carry on_mouse_exit"
            );
        }
    }

    #[test]
    fn pomo_click_stays_mutating_hover_only_adds_a_passive_peek() {
        // Hover must never trigger a mutating action — pomo's on_click_left
        // still fires the mutating `pomo click` verb and nothing else.
        //
        // Correction, T-hover follow-up: on_click_left used to also toggle
        // the popup (`; ironbar bar {bar} toggle-popup pomo`) — dropped,
        // same reason claudebar/clock/date lost their own toggle. Hover
        // alone shows/hides the popup now; the click stays a pure mutation.
        //
        // T15: middle-click retired — mute moved to right, reset dropped
        // off the bar (still reachable as `mango-bard pomo reset -q` from
        // the CLI).
        //
        // T-next (item 3): `pomo` is part of `rightcenter_modules()`, which
        // lives in `end` on a real per-monitor bar now.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let pomo = end.iter().find(|m| m["name"] == "pomo").unwrap();
        assert_eq!(pomo["on_click_left"], json!("mango-bard pomo click -q"));
        assert!(pomo.get("on_click_middle").is_none());
        assert_eq!(pomo["on_click_right"], json!("mango-bard pomo mute -q"));
        assert!(pomo["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("hover enter bar-eDP-1 pomo"));
        assert!(pomo["on_mouse_exit"]
            .as_str()
            .unwrap()
            .contains("hover exit bar-eDP-1 pomo"));
    }

    #[test]
    fn toggle_popup_modules_never_send_toggle_popup_from_on_click_left() {
        // cpu/memory/docker/battery/volume/bluetooth/claudebar/clock/date:
        // every pure detail popup with no mutating click of its own. A
        // plain or refresh-then toggle-popup on_click_left would fight
        // hover's own open/close (`ipc.rs::show_popup` sends `toggle_popup`
        // under the hood, so a click and a hover-open racing would toggle
        // each other). `pomo` is the one exception — its click has to stay,
        // see `pomo_click_stays_mutating_hover_only_adds_a_passive_peek`.
        //
        // T15: cpu/memory/battery/date/volume now carry a real
        // `on_click_left` — an external-process launch (btop, powermode,
        // calendar, mixer) moved here off the retired middle-click. That
        // launch never sends `toggle-popup`, so it cannot race hover the
        // way the old refresh-then-toggle pattern could; the test below
        // checks for that string instead of requiring the key's absence.
        // T-next (item 3): cpu/memory/docker/battery/claudebar are part of
        // `leftcenter_modules()` (now in `start`); clock/date are part of
        // `rightcenter_modules()` (now in `end` on a real per-monitor bar).
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        for name in ["cpu", "memory", "docker", "battery", "claudebar"] {
            let m = start.iter().find(|m| m["name"] == name).unwrap();
            if let Some(click) = m.get("on_click_left").and_then(|v| v.as_str()) {
                assert!(
                    !click.contains("toggle-popup"),
                    "{name}'s on_click_left must not send toggle-popup: {click}"
                );
            }
        }
        for name in ["clock", "date"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
            if let Some(click) = m.get("on_click_left").and_then(|v| v.as_str()) {
                assert!(
                    !click.contains("toggle-popup"),
                    "{name}'s on_click_left must not send toggle-popup: {click}"
                );
            }
        }
        // T19: wifi/eth/netsec join volume/bluetooth — their on_click_left
        // is a real shell script launch (wifi-menu.sh/eth-toggle.sh/
        // net.sh), never a popup toggle, so it can't race hover either.
        for name in ["volume", "bluetooth", "wifi", "eth", "netsec"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
            if let Some(click) = m.get("on_click_left").and_then(|v| v.as_str()) {
                assert!(
                    !click.contains("toggle-popup"),
                    "{name}'s on_click_left must not send toggle-popup: {click}"
                );
            }
        }
        // bluetooth/docker/claudebar/clock carry no on_click_left at all —
        // still true, and worth pinning so a future edit that adds one
        // notices this test instead of sliding past it silently.
        for name in ["docker", "claudebar"] {
            let m = start.iter().find(|m| m["name"] == name).unwrap();
            assert!(m.get("on_click_left").is_none());
        }
        let clock = end.iter().find(|m| m["name"] == "clock").unwrap();
        assert!(clock.get("on_click_left").is_none());
        let bt = end.iter().find(|m| m["name"] == "bluetooth").unwrap();
        assert!(bt.get("on_click_left").is_none());
    }

    #[test]
    fn workspace_pill_right_click_toggle_popup_survives_as_a_fallback() {
        // Unlike the six modules above, the workspace pills' right-click
        // toggle-popup is kept deliberately — see workspace_pills()'s own
        // doc comment: it isn't in tension with hover (a different
        // gesture), so removing a working manual fallback for no reason
        // would not be the smallest diff.
        let cfg = build(&["eDP-1".to_string()]);
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let ws1 = center
            .iter()
            .find(|m| m["name"] == ws_module("eDP-1", 1))
            .unwrap();
        let click = ws1["on_click_right"].as_str().unwrap();
        assert!(click.contains("toggle-popup"));
        assert!(click.contains("bar-eDP-1"));
    }
}
