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
    popup_multi(&[tip_key], bar_name)
}

/// T29: generalises `popup()` to N sections for a stacked module
/// (`sysload`/`devload`) — one top-level popup per stack, since a nested
/// module's own popup never registers (see `sysload_module()`'s own doc
/// comment for why the stack can't just be N popup-bearing modules). Each
/// `tip_key` gets its own optional title, its own body, its own optional
/// hint, and sections after the first are preceded by their own
/// `popup-sep` — so two sections read as two blocks, not one merged one.
fn popup_multi(tip_keys: &[&str], bar_name: &str) -> Value {
    let mut widgets = Vec::new();
    for (i, tip_key) in tip_keys.iter().enumerate() {
        if i > 0 {
            widgets.push(json!({ "type": "box", "class": "popup-sep" }));
        }
        if crate::tooltip::TITLED.contains(tip_key) {
            widgets.push(
                json!({ "type": "label", "class": "popup-title", "label": format!("#{tip_key}_title") }),
            );
            widgets.push(json!({ "type": "box", "class": "popup-sep" }));
        }
        widgets.push(json!({ "type": "label", "label": format!("#{tip_key}") }));
        if let Some((_, hint)) = crate::tooltip::HINTS.iter().find(|(k, _)| k == tip_key) {
            widgets.push(json!({ "type": "box", "class": "popup-sep" }));
            widgets.push(json!({ "type": "label", "class": "popup-hint", "label": hint }));
        }
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
    // T28 tray drawer — see `tray_toggle_module()`'s own doc comment.
    // Starts closed: the drawer's whole point is a collapsed resting
    // state.
    defaults.insert("tray_open".into(), json!("false"));
    // T28 keepass vars — see keepass.rs.
    defaults.insert("kp_text".into(), json!(""));
    defaults.insert("kp_tip".into(), json!(""));
    // T28 archupdate vars — see archupdate.rs. `au_show` defaults "false":
    // hidden until a real pending count is known, same reasoning as
    // `hotspot_show`'s own default above.
    defaults.insert("au_show".into(), json!("false"));
    defaults.insert("au_text".into(), json!(""));
    defaults.insert("au_tip".into(), json!(""));
    // T28 music vars — see music.rs. `music_on` defaults "false": hidden
    // until a real track is known, same reasoning as `au_show` above.
    defaults.insert("music_on".into(), json!("false"));
    defaults.insert("music_text".into(), json!(""));
    defaults.insert("music_tip".into(), json!(""));
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

    // T23: `center` now holds the time block (clock/date/pomo) on every
    // bar, real or fallback — see `time_modules()`'s own doc comment for why
    // this reverses T-next (item 3)'s deliberate emptying of `center`. The
    // reversal is safe: putting the tag pills in `start` instead anchors
    // them to a fixed X, which is what that stage was actually trying to
    // buy and what INV-4 (statusbar-layout.md) demands directly — a fixed-
    // width `center` (INV-6) no longer depends on `center` being pill-only,
    // it depends on `time_modules()` itself being fixed-width, which it is.
    let mut monitors_map = serde_json::Map::new();
    for (name, slug) in monitors.iter().zip(&slugs) {
        let bar_name = format!("bar-{name}");
        let mut bar = serde_json::Map::new();
        bar.insert("name".into(), json!(bar_name));
        // T8c: = config.jsonc:4's `"height": 40`. Ironbar's own
        // default is 42 (schema default, `BarConfig.height`), and
        // GTK treats it as a minimum, not a fixed value — it grows
        // to fit taller content regardless. Set for parity with
        // waybar's own height, not as a hard cap.
        bar.insert("height".into(), json!(40));
        // T8 cutover: ironbar takes the top edge waybar used to own
        // (bars.sh killed, not restarted). No separate popup-
        // direction setting exists (`--print-schema` confirmed at
        // T0/S5) — ironbar derives it from `position` itself.
        bar.insert("position".into(), json!("top"));
        // T9: raised from the schema default (5) — the only distance
        // lever `PopupConfig`/`BarConfig` expose for "popup sits too
        // close to the button" (no per-widget offset field exists).
        bar.insert("popup_gap".into(), json!(12));
        if name.starts_with("HEADLESS") {
            // T-headless-strip: a HEADLESS-* output is a VNC capture
            // surface, not a real screen edge — clock/tray/audio/etc.
            // have no viewer to serve. It also has no tags of its own
            // worth showing: whatever the remote viewer opened there is
            // already on screen. The bar instead carries a
            // remote-control strip — one private pill (`headless_pill`),
            // then every physical monitor's name and nine pills
            // (`remote_pills`), whose click pulls that monitor's tag onto
            // this output (`remote.sh --pull`) instead of viewing it.
            // `remote_pills` reuses `ws_pill`'s module names, so
            // `mango-bard`'s per-monitor `@class`/label sends
            // (`mango.rs::apply()`) already reach these copies with no
            // daemon change — `ironbar style add-class` fans a class out
            // to every module with that name, and the live bar already
            // repeats plain module names (`clock`, `battery`, ...) across
            // bar-DP-1/bar-eDP-1/bar-default the same way. No
            // `center`/`end`: omitted, not empty arrays, since nothing
            // populates them here.
            let mut start = vec![headless_pill()];
            for (pmon, pslug) in monitors.iter().zip(&slugs) {
                if pmon.starts_with("HEADLESS") {
                    continue;
                }
                start.push(mon_label(pmon));
                start.extend(remote_pills(pmon, pslug, &bar_name));
            }
            bar.insert("start".into(), json!(start));
        } else {
            bar.insert(
                "start".into(),
                json!(start_modules(&bar_name, Some((name, slug)))),
            );
            bar.insert("center".into(), json!(time_modules(&bar_name)));
            bar.insert("end".into(), json!(end_modules(&bar_name)));
        }
        monitors_map.insert(name.clone(), Value::Object(bar));
    }

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
        // No `mon_slug` — T0 spike S3's unlisted-output case has no
        // monitor name/slug to build tag pills from, so `start_modules`
        // simply omits them here (see its own doc comment).
        "start": start_modules("bar-default", None),
        "center": time_modules("bar-default"),
        "end": end_modules("bar-default"),
        "monitors": Value::Object(monitors_map),
    })
}

/// `end` row, grouped by domain per statusbar-layout.md §3.3-3.8: tray
/// (leads, INV-1 growth end) → resources → darkmode/inhibit → audio →
/// connectivity →
/// session. Each sub-group is one saccade's worth of ambient information
/// (proximity/common region — see the spec's own §Rationale), not a flat
/// list of 18 unrelated pills.
///
/// T23: this replaces T-next (item 3)'s `rightcenter_modules()`-prepended
/// shape. `center` now carries the time block directly (`time_modules()`,
/// wired in `build()`) instead of `end` borrowing it — see that function's
/// own doc comment for why the reversal of T-next item 3 is safe.
fn end_modules(bar_name: &str) -> Vec<Value> {
    // T28: tray, its own toggle, then the promoted pill — see
    // `tray_module()`'s own doc comment for why the drawer needs it.
    // T29: `archupdate` moved out of this block into `devload`'s own
    // second row (shares it with `docker` — see `devload_module()`'s doc
    // comment) — `keepass` is the only pill still promoted here.
    //
    // T29 follow-up: the drawer itself is OFF by default now
    // (`TRAY_DRAWER_ENABLED`) — user preference is the full tray icon list
    // always visible, no hide-behind-toggle. `tray_toggle_module()` is
    // simply not added when disabled; `tray_module()` itself drops its own
    // `show_if` in the same case (see its own doc comment).
    let mut end = vec![tray_module()];
    if TRAY_DRAWER_ENABLED {
        end.push(tray_toggle_module());
    }
    end.push(keepass_module(bar_name));
    end.extend(resource_modules(bar_name));
    end.extend(tools_modules(bar_name));
    end.extend(audio_modules(bar_name));
    end.extend(net_modules(bar_name));
    end.push(hotspot_module(bar_name));
    end.push(remote_module(bar_name));
    end.push(bluetooth_module(bar_name));
    end.push(power_module());
    end
}

/// Resource-monitor block (statusbar-layout.md §3.4): cpu, memory, docker,
/// battery, claudebar (+ archupdate) — one glance-only "is this machine
/// healthy" group, general to specific. Renamed from `leftcenter_modules`
/// (T23): `music` moved out to `audio_modules` (§3.6, grouped with volume/
/// mic instead — proximity by domain, not by original waybar slot), and
/// this group's home row changed from `start` to `end` (see
/// `end_modules()`'s own doc comment) — waybar's own `group/leftcenter`
/// order this once mirrored no longer applies to either its position or its
/// membership.
///
/// T29: `cpu`+`memory` and `claudebar`+`docker`(+`archupdate`) each became
/// one stacked module (`sysload_module()`/`devload_module()`) — see either
/// function's own doc comment for why a plain two-row `custom` module,
/// rather than nesting native modules, is the only shape that keeps every
/// row's own state classes reachable. `battery` is unaffected: it was
/// already a single row and stays one.
fn resource_modules(bar_name: &str) -> Vec<Value> {
    let mut m = vec![sysload_module(bar_name)];
    m.extend(power_modules(bar_name));
    m.push(devload_module(bar_name));
    m
}

/// `center`'s whole content (statusbar-layout.md §3.2): clock, date, pomo —
/// nothing else. T23 reverses T-next (item 3)'s decision to empty `center`
/// down to the tag pills: that stage moved clock/date/pomo/colorpicker/
/// darkmode/snip/inhibit out of `center` because the group carried 15
/// modules of mixed width and GTK centres `center` as one block, so any of
/// the other 6 changing width shifted the pills sideways. The tag pills
/// moved to `start` instead this stage (see `start_modules()`'s own doc
/// comment) — a fixed X for the pills no longer depends on `center` being
/// pill-only, it depends on `start` anchoring them right after `spark`,
/// which INV-4 (statusbar-layout.md) demands directly. `center` can hold
/// the time block again because `time_modules()` is fixed-width on its own
/// (INV-5/INV-6) — three modules, no variable-width member — not because
/// `center` merely holds fewer things.
fn time_modules(bar_name: &str) -> Vec<Value> {
    let mut m = clock_pill(bar_name);
    m.push(pomo_pill(bar_name));
    m
}

/// T31: the T23 tools drawer is gone — the user judged the "…" trigger a
/// waste of space, and wants the TOOLS THEMSELVES back on the bar, not
/// hidden: colorpicker, darkmode and snip return as standalone pills in
/// the T23 reading order, `inhibit` follows unchanged. (The first T31 cut
/// deleted colorpicker/snip as keybind duplicates — user feedback
/// reversed that: "remove the drawer" meant the reveal mechanism only.)
fn tools_modules(bar_name: &str) -> Vec<Value> {
    vec![
        colorpicker_module(),
        darkmode_module(),
        snip_module(),
        inhibit_module(bar_name),
    ]
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

/// `start` row (statusbar-layout.md §3.1): launcher → tags → focus.
///
/// T23: `mon_slug` (monitor name + its ws-var slug) is threaded through so
/// [`workspace_pills`] can sit here, between `spark` and `win` — this
/// reverses T-next (item 3)'s move of the tag pills into `center`; see
/// `time_modules()`'s own doc comment for why that reversal is safe.
/// `None` on the fallback bar (T0 spike S3's unlisted-output case: no
/// monitor name/slug exists to build tag vars from), so the fallback bar
/// renders `spark`/`win` with no tag block, same shape it already had.
/// `win` stays last — INV-1 (statusbar-layout.md): it is the one
/// unbounded-width field in this row, so it must sit at the growth end,
/// displacing nothing to its left.
fn start_modules(bar_name: &str, mon_slug: Option<(&str, &str)>) -> Vec<Value> {
    let mut m = vec![spark_module()];
    if let Some((mon, slug)) = mon_slug {
        m.extend(workspace_pills(mon, slug, bar_name));
    }
    m.push(window_module(bar_name));
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
///
/// T28: gated behind `tray_open` — `TrayModule` has no per-item filter
/// (only `icon_size`/`direction`/`prefer_theme_icons` plus the common
/// options; every item gets `.item`, none gets a name — confirmed against
/// the vendored `TrayModule` schema and `TrayMenu::new`), so "important vs
/// hidden" cannot be expressed inside the tray itself. The two items worth
/// a permanent glance (KeePassXC, Arch-Update) are rebuilt as their own
/// pills instead — `keepass_module`, right after `tray_toggle_module`
/// below, and Arch-Update's own pending count (T29: folded into
/// `devload_module`'s docker row, not its own top-level pill any more) —
/// the whole native tray collapses behind the toggle for the rest.
/// `on_click_left` replaces the default SNI activate for every item at
/// once (ironbar substitutes `{address}`) —
/// `tray-click.sh` jumps to an already-open window before falling back to
/// activate, so this is a net gain for every tray app, not a regression;
/// the one accepted cost is NordVPN's left-click menu moving to
/// right-click (it is the only item with `ItemIsMenu: true`, confirmed
/// live via `busctl --user get-property ... ItemIsMenu`).
///
/// T29 follow-up: `TRAY_DRAWER_ENABLED` (false by default) turns the
/// `show_if` gate off entirely rather than deleting it — user preference
/// is every tray icon always visible, no hide-behind-toggle, but the
/// mechanism (this gate, `tray_toggle_module()`, `tray-drawer.sh`, the
/// `.traytoggle` CSS) stays intact for a future re-enable: flip the one
/// constant back to `true` and all four still work unchanged.
const TRAY_DRAWER_ENABLED: bool = false;

fn tray_module() -> Value {
    let mut m = json!({
        "type": "tray",
        "name": "tray",
        "class": "tray",
        "icon_size": 16,
        "transition_type": "slide_end",
        "on_click_left": "~/.config/ironbar/scripts/tray-click.sh {address}"
    });
    if TRAY_DRAWER_ENABLED {
        m["show_if"] = json!("#tray_open");
    }
    m
}

/// T28: the drawer's own trigger, right after `tray` so opening it grows
/// away from the pointer and the trigger's own X never moves (INV-1). One
/// plain Unicode glyph (vertical ellipsis, "more"), not a Nerd Font
/// codepoint: plain punctuation renders correctly through the base
/// `* { font-family }` stack with zero new verification needed (T31: the
/// tools drawer whose horizontal-ellipsis trigger this once contrasted
/// with is gone).
fn tray_toggle_module() -> Value {
    json!({
        "type": "custom",
        "name": "traytoggle",
        "class": "traytoggle",
        "bar": [ { "type": "label", "label": "\u{22ee}" } ],
        "tooltip": "Tray icons",
        "on_click_left": "~/.config/ironbar/scripts/tray-drawer.sh"
    })
}

/// KeePassXC lock-state pill (T28, see keepass.rs) — promoted out of the
/// tray drawer since lock state is glance-worthy. `on_click_left` reuses
/// the exact named scratchpad already bound to `Alt+k` in mango/config.conf
/// (`toggle_named_scratchpad,org.keepassxc.KeePassXC,none,keepassxc`).
fn keepass_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "keepass",
        "class": "keepass",
        "bar": [ { "type": "button", "label": "#kp_text" } ],
        "popup": popup("kp_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} keepass -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} keepass -q"),
        "on_click_left": "mmsg dispatch toggle_named_scratchpad,org.keepassxc.KeePassXC,none,keepassxc"
    })
}

// T29: `archupdate` (T28, see archupdate.rs) stopped being its own
// top-level pill — its `au_text`/`au_show`/`au_tip` ironvars and its click
// are now a second cell on `devload`'s docker row. See
// `devload_module()`'s own doc comment.

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
/// pattern (sysload_module's own doc comment explains why a plain `on_click`
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
            // refresh-then-toggle-popup is gone, same reasoning
            // `devload_module()`'s own doc comment gives for its claude
            // row — a plain detail popup with no mutating click has
            // nothing left for a click to do
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
/// Click layout follows T4's rule (sysload_module's own doc comment): left opens
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

/// Audio block (statusbar-layout.md §3.6): music, volume, mic — one domain
/// by proximity/common region ("what's playing, and what is the machine
/// doing with sound"). T23: `music_module()` moved here from `start`'s old
/// `leftcenter_modules()`/`resource_modules()` group — see this function's
/// own doc comment for why the resource block is machine-health only now.
///
/// Volume/mic: T4 (see audio.rs). Bar-global, like `net_modules()` —
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
        music_module(bar_name),
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
/// `sysload_module`, to build the `hover enter|exit <bar> <widget>` commands.
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
/// `power_modules`/`audio_modules` already established
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
/// T29: `cpu` and `memory` merged into one module — **why not two nested
/// `custom` modules:** a nested `WidgetOrModule::Module` is a real option
/// in ironbar's schema, but `custom/mod.rs::add_to` (vendored source)
/// discards the `ModuleRef` it gets back, and `style add_class`/
/// `remove_class` resolve a module by name only through `Bar::modules()` —
/// populated solely from the top-level `start`/`center`/`end` arrays
/// (`bar.rs::add_modules`). A nested `cpu` module would answer "Module not
/// found" for every `@class/cpu` push: every gauge level, every warning
/// colour, dead. Plain widgets inside one module sidestep this: the class
/// vars land on the module node and CSS descends from there.
///
/// **One popup covers both resources** (`popup_multi`) — a nested widget's
/// own popup would hit the identical "module not found" problem via
/// `ipc.rs::show_popup`'s `widget_name` lookup.
///
/// T31: the bar root became ONE `button` wrapping the whole stack. Hover
/// was dead on the T29 shape: working pills (`battery`, `keepass`) have a
/// `button` bar root — the module-level `on_mouse_enter` fires from it —
/// while T29's stack rooted in a windowless `box` whose visible area was
/// covered by two nested `button` rows, and a nested button's own GDK
/// event window swallows every crossing before the module handler sees
/// one: `mango-bard stats` showed `cpu_detail_builds:0` over hours of
/// uptime while single-button pills incremented. The rows are plain
/// `box`/`label` widgets now — windowless, so events fall through to the
/// root button — and the one `btop` click moves onto that button. A
/// `button` root also makes `ipc.rs`'s `buttons.first()` anchor the
/// popup on the pill itself.
///
/// T31 follow-up: the first cut flattened the stack to one wide row; the
/// user wants the cpu gauge ON TOP of the memory gauge — the vertical
/// two-row shape returns, only the widget types changed (buttons ->
/// windowless boxes) to keep hover alive.
///
/// The gauges carry distinct classes (`gauge-cpu`/`gauge-mem`) because
/// both sit under the one module node, so `.clNN .gauge` would hit both;
/// the labels likewise (`cpu-ico`/`mem-ico`) so the warning colour rules
/// have a per-resource target where `.row-cpu`/`.row-mem` used to be.
/// cpu.rs/memory.rs are untouched: their `@class/sysload#*` pushes always
/// landed on the module node, and still select through it.
///
/// `valign: "center"` on the stack box and on each gauge — `BoxWidget`'s
/// own schema defaults `valign` to `"fill"`, which packs the rows'
/// combined natural height at the TOP of the bar's 40px (T29 follow-up)
/// and stretches a gauge to its row's full height (T28).
fn sysload_module(bar_name: &str) -> Value {
    fn row(ico_class: &str, text_var: &str, gauge_class: &str) -> Value {
        json!({ "type": "box", "widgets": [
            { "type": "label", "class": ico_class, "label": format!("#{text_var}") },
            { "type": "box", "class": gauge_class, "valign": "center" }
        ] })
    }
    json!({
        "type": "custom",
        "name": "sysload",
        "class": "sysload",
        "bar": [ { "type": "button", "widgets": [
            { "type": "box", "orientation": "vertical", "valign": "center", "widgets": [
                row("cpu-ico", "cpu_text", "gauge-cpu"),
                row("mem-ico", "mem_text", "gauge-mem")
            ] }
        ], "on_click_left": "kitty --class mango-monitor -e btop" } ],
        "popup": popup_multi(&["cpu_tip", "mem_tip"], bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} sysload -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} sysload -q")
    })
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
///
/// T29: gains a third widget, `.gauge-cap` — a small filled square right of
/// the gauge so the pair reads as a battery (rectangle + terminal nub)
/// instead of a plain rounded bar. Battery only: `sysload_module()`'s two
/// gauges stay bare, this is what visually marks battery as the one gauge
/// that means "charge remaining" rather than "load". `bat_text` itself
/// drops its `NN%` (power.rs) — the gauge now carries the level the same
/// way T28 already made it carry cpu/memory's.
fn power_modules(bar_name: &str) -> Vec<Value> {
    vec![json!({
        "type": "custom",
        "name": "battery",
        "class": "battery",
        // T28: same `widgets`-not-`label` reasoning as sysload_module().
        // T31: `.gauge` gains a child `.gauge-fill` that carries the
        // level gradient (`.pNN .gauge-fill`, style.css). One node used
        // to carry border AND fill, so the charging pulse (an opacity
        // animation) dimmed the outline too; a separate fill node lets
        // only the charge level flash. The child sits inside `.gauge`'s
        // 2px border automatically (GTK borders inset content).
        "bar": [ { "type": "button", "widgets": [
            { "type": "label", "label": "#bat_text" },
            { "type": "box", "class": "gauge", "valign": "center", "widgets": [
                { "type": "box", "class": "gauge-fill" }
            ] },
            { "type": "box", "class": "gauge-cap", "valign": "center" }
        ] } ],
        "popup": popup("bat_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} battery -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} battery -q"),
        "on_click_left": "~/.config/mango/scripts/powermode.sh toggle",
        "on_click_right": "kitty --class mango-monitor -e sudo -n powertop"
    })]
}

/// T29: `claudebar` and `docker` merged into one two-row stack, the same
/// shape and for the same reason as `sysload_module()` (see its own doc
/// comment for the nested-module class-update dead end and the
/// one-popup-per-stack consequence) — read that comment first, it is not
/// repeated here. `archupdate` (T28, see archupdate.rs) folds in as a
/// second cell on the docker row rather than getting a third row of its
/// own: three 40px-tall rows read as noise, and a pending-updates count is
/// no busier than docker's own count, so the two share a row as two
/// buttons side by side (`show_if: "#au_show"` still hides the cell at
/// zero, exactly as the old standalone pill did).
///
/// Clicks carry over unchanged: right-click on the claude row opens the
/// usage page (T6c); right-click on the docker cell opens docker-menu.sh
/// (T6b D4); left-click on the updates cell runs `arch-update` (T28). None
/// of the three has a left-click toggle any more — carried over from the
/// old `claudebar_module`'s own fix: `ipc.rs::show_popup` sends
/// `toggle_popup` under the hood, so a leftover click toggle on a
/// hover-opened, no-mutating-click popup would just close what hover had
/// just opened.
/// T31: the module-level `on_mouse_enter`/`on_mouse_exit` pair is
/// duplicated onto every nested `button`. The stack keeps its `box` bar
/// root (three distinct click targets rule out sysload's single-button
/// merge), and a `box` root is windowless in GTK3 — the nested buttons'
/// own GDK event windows swallow every crossing, so the module-level
/// handlers alone never fire (`mango-bard stats`: zero detail builds for
/// stacked modules, non-zero for every button-root pill). Widget-level
/// mouse handlers are already proven live on `popup()`'s hold/release
/// box (T22). Pointer hand-offs between the rows fire exit+enter pairs,
/// but both route into main.rs's hover state machine, where a re-enter
/// within `HOVER_HIDE_GRACE` supersedes the pending grace-hide
/// (`hover_enter`'s own doc comment) — the popup does not flicker.
fn devload_module(bar_name: &str) -> Value {
    let enter = format!("mango-bard hover enter {bar_name} devload -q");
    let exit = format!("mango-bard hover exit {bar_name} devload -q");
    json!({
        "type": "custom",
        "name": "devload",
        "class": "devload",
        // T29 follow-up: `valign: "center"` — `BoxWidget`'s own schema
        // defaults `valign` to `"fill"`, which packs the rows' combined
        // natural height at the TOP of the bar's 40px allocation instead
        // of centering in it.
        "bar": [ { "type": "box", "orientation": "vertical", "valign": "center", "widgets": [
            {
                "type": "button",
                "class": "row-claude",
                "label": "#claude_text",
                "on_click_right": "xdg-open https://claude.ai/settings/usage",
                "on_mouse_enter": enter.clone(),
                "on_mouse_exit": exit.clone()
            },
            { "type": "box", "class": "row-svc", "widgets": [
                {
                    "type": "button",
                    "class": "cell-docker",
                    "label": "#docker_text",
                    "on_click_right": "~/.config/ironbar/scripts/docker-menu.sh",
                    "on_mouse_enter": enter.clone(),
                    "on_mouse_exit": exit.clone()
                },
                {
                    "type": "button",
                    "class": "cell-au",
                    "show_if": "#au_show",
                    "label": "#au_text",
                    "on_click_left": "kitty --class mango-monitor -e arch-update",
                    "on_mouse_enter": enter.clone(),
                    "on_mouse_exit": exit.clone()
                }
            ] }
        ] } ],
        "popup": popup_multi(&["claude_tip", "docker_tip", "au_tip"], bar_name),
        "on_mouse_enter": enter,
        "on_mouse_exit": exit
    })
}

/// Hotspot pill: T6b (see hotspot.rs). Bar-global, like `keepass_module()`.
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
/// (style.css's `.remote.active`/`.remote.partial`). `on_click_left` toggles
/// VNC only (turning it on also starts kdeconnectd; turning it off leaves
/// kdeconnectd running); `on_click_right` toggles kdeconnectd only. Tag
/// pull cycling moved off the pill onto the SUPER+CTRL keybinds — a
/// three-state pill (VNC/KDE Connect/pull) has no room left for a third
/// click.
///
/// T-remote-popup: the popup is the plain `popup()` stack again, same as
/// every other simple pill. A deleted helper used to append a clickable
/// per-monitor tag grid below the body. The grid was dropped on user report
/// — a popup that only opens on hover over a 20px pill is not a place a tag
/// button can be reached in practice. The popup is read-only status now.
fn remote_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "remote",
        "class": "remote",
        "bar": [ { "type": "button", "label": "#remote_text" } ],
        "popup": popup("remote_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} remote -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} remote -q"),
        "on_click_left": "~/.config/ironbar/scripts/remote.sh --toggle-vnc",
        "on_click_right": "~/.config/ironbar/scripts/remote.sh --toggle-kdeconnect"
    })
}

/// T28: `custom` module fed by music.rs, replacing the native `music`
/// module this used to be (config.jsonc:62-69's own `mpris` built-in,
/// ported at T8c). The native module renders through GTK4's
/// `set_label_escaped` (confirmed against the vendored source,
/// `modules/music/mod.rs`), which can only ever show plain text — the
/// two-line dim-app/plain-title markup `mango.rs::window_text` uses for the
/// window pill needs a real `<span>` string, which only a daemon-fed
/// ironvar can supply. `show_if` replaces the old `truncate`-only emptiness
/// handling: the pill now disappears entirely with nothing loaded, instead
/// of reserving space for an empty string (`music.rs`'s own `music_on`
/// doc comment).
fn music_module(bar_name: &str) -> Value {
    json!({
        "type": "custom",
        "name": "music",
        "class": "music",
        "show_if": "#music_on",
        "bar": [ { "type": "button", "label": "#music_text" } ],
        "popup": popup("music_tip", bar_name),
        "on_mouse_enter": format!("mango-bard hover enter {bar_name} music -q"),
        "on_mouse_exit": format!("mango-bard hover exit {bar_name} music -q"),
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

/// Darkmode toggle: T6b (see darkmode.rs).
///
/// T23 nested it inside the tools drawer; T31 removed the drawer (see
/// `tools_modules()`'s own doc comment) and this became a standalone
/// `custom` module again — `button` bar root like `inhibit_module()`, the
/// shape whose click and hover are proven to work. The static `tooltip`
/// returns: the icon is always visible again, so it needs a name on
/// hover (ironbar schema: `tooltip` is `string|null`, static strings need
/// no `{{script}}`; no `on_mouse_enter` here, so no conflict with the
/// static-tooltip-xor-hover-popup rule).
fn darkmode_module() -> Value {
    json!({
        "type": "custom",
        "name": "darkmode",
        "class": "darkmode",
        "bar": [ { "type": "button", "label": "#dark_icon" } ],
        "tooltip": "Toggle light/dark",
        "on_click_left": "~/.config/ironbar/scripts/darkmode.sh --toggle"
    })
}

/// Color picker: standalone pill again (T31, see `tools_modules()`'s own
/// doc comment — its whole T23 drawer life is over). Same shape as
/// `power_module()`: `label` bar root (windowless, so the module-level
/// click lands — proven by power's own working click), static tooltip.
/// Glyph history (U+E3B8 -> U+F1FB -> U+F020B md-eyedropper_variant) is
/// in IRONBAR.md's T8b/T19 entries.
fn colorpicker_module() -> Value {
    json!({
        "type": "custom",
        "name": "colorpicker",
        "class": "colorpicker",
        "bar": [ { "type": "label", "label": "\u{f020b}" } ],
        "tooltip": "Color picker",
        "on_click_left": "hyprpicker -a"
    })
}

/// Screenshot pill: standalone again (T31, same reversal as
/// `colorpicker_module()` above). Both clicks route through
/// screenshot.sh so each mode saves + copies + notifies the same way
/// (T20); the tooltip carries the discoverability the drawer dropped.
/// Glyph U+F0E5A (md-monitor_screenshot) — T19's one-family sweep.
fn snip_module() -> Value {
    json!({
        "type": "custom",
        "name": "snip",
        "class": "snip",
        "bar": [ { "type": "label", "label": "\u{f0e5a}" } ],
        "tooltip": "Screenshot — left: region, right: window",
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
///
/// One numbered tag pill's module JSON, for `mon`'s tag `n`. Shared by
/// [`workspace_pills`] (the physical bar — clicks/scrolls act on mango's
/// focused monitor, `scrollable: true`) and [`remote_pills`] (the headless
/// bar's copy of a physical monitor's pills — click pulls the tag instead,
/// `scrollable: false`; see that function's own doc comment).
/// `bar_name` is the bar the pill is actually rendered on, not necessarily
/// `mon`'s own bar — `remote_pills` renders `mon`'s pills on the headless
/// bar, so the module's `popup`/hover/toggle-popup targets must name that
/// bar, not `mon`'s.
/// `gated` sets `show_if: "#ws_<slug>_tags"` (hides the pill while `mon`
/// is in overview). `workspace_pills` wants that — a monitor's own pills
/// should vanish on its own overview screen. `remote_pills` passes
/// `gated: false`: gating the headless strip's copy on a *different*
/// monitor's overview state would reflow the whole strip (the other
/// monitor's block sliding over) in response to a screen the remote
/// viewer can't see — see `remote_pills`'s own doc comment.
fn ws_pill(
    mon: &str,
    slug: &str,
    n: u64,
    bar_name: &str,
    on_click_left: String,
    scrollable: bool,
    gated: bool,
) -> Value {
    let module = ws_module(mon, n);
    let mut pill = serde_json::Map::new();
    pill.insert("type".into(), json!("custom"));
    pill.insert("name".into(), json!(module));
    pill.insert("class".into(), json!("ws"));
    if gated {
        pill.insert("show_if".into(), json!(format!("#{}", var_tags(slug))));
    }
    pill.insert(
        "bar".into(),
        json!([ { "type": "button", "widgets": [
            { "type": "label", "justify": "center", "label": format!("#{}", var_lbl(slug, n)) }
        ] } ]),
    );
    pill.insert("popup".into(), popup(&var_tip(slug, n), bar_name));
    pill.insert("on_click_left".into(), json!(on_click_left));
    // T-hover: hover opens/closes the popup; right-click's own
    // toggle-popup stays too, as a harmless manual fallback — unlike
    // cpu/memory/docker/battery/volume/bluetooth, this gesture isn't in
    // tension with hover (it's a different click, not the redundant one
    // hover replaced). No refresh is dispatched on hover-open here:
    // mango.apply() already keeps every tag's tip live off its own mmsg
    // event stream, so there is nothing stale to lazily rebuild.
    pill.insert(
        "on_mouse_enter".into(),
        json!(format!("mango-bard hover enter {bar_name} {module} -q")),
    );
    pill.insert(
        "on_mouse_exit".into(),
        json!(format!("mango-bard hover exit {bar_name} {module} -q")),
    );
    pill.insert(
        "on_click_right".into(),
        json!(format!("ironbar bar {bar_name} toggle-popup {module}")),
    );
    if scrollable {
        pill.insert("on_scroll_up".into(), json!("mmsg dispatch viewtoleft,0"));
        pill.insert(
            "on_scroll_down".into(),
            json!("mmsg dispatch viewtoright,0"),
        );
    }
    // T-next (item 5): default `show_if` transition is `slide_start` — a
    // left-to-right slide, which read as the pill group shuffling
    // sideways rather than a tag simply appearing/disappearing. Ironbar
    // has no slide-from-top option (`--print-schema`: slide_start/
    // slide_end/crossfade/none only), so a fade is the closest match.
    pill.insert("transition_type".into(), json!("crossfade"));
    Value::Object(pill)
}

fn workspace_pills(mon: &str, slug: &str, bar_name: &str) -> Vec<Value> {
    let mut pills: Vec<Value> = (1..=TAG_COUNT)
        .map(|n| {
            ws_pill(
                mon,
                slug,
                n,
                bar_name,
                format!("mmsg dispatch view,{n},0"),
                true,
                true,
            )
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

/// T-headless-strip: the headless bar's copy of `mon`'s nine tag pills.
/// Same module names as `mon`'s own bar (see `ws_pill`'s doc comment for
/// why that's safe), but `on_click_left` pulls the tag onto the headless
/// output (`remote.sh --pull`) instead of viewing it, and there is no
/// scroll and no overview pill: `viewtoleft`/`viewtoright`/
/// `toggleoverview` act on mango's *focused* monitor, which on this bar is
/// never `mon` — they would move the viewer's own headless view, not
/// `mon`'s. Not gated on `mon`'s own `ws_<slug>_tags` either — the strip
/// shows every physical monitor at once, so hiding `mon`'s block whenever
/// `mon` happens to be in overview would reflow the neighbouring block
/// sideways for a reason the viewer can't see (they aren't looking at
/// `mon`'s screen).
fn remote_pills(mon: &str, slug: &str, bar_name: &str) -> Vec<Value> {
    (1..=TAG_COUNT)
        .map(|n| {
            ws_pill(
                mon,
                slug,
                n,
                bar_name,
                format!("~/.config/ironbar/scripts/remote.sh --pull {mon} {n}"),
                false,
                false,
            )
        })
        .collect()
}

/// Static screen-name label ahead of `mon`'s pills on the headless bar's
/// remote-control strip. No ironvar: a connector name never changes at
/// runtime, so there is nothing for `mango-bard` to keep live here.
fn mon_label(mon: &str) -> Value {
    json!({
        "type": "custom",
        "name": format!("ws-{mon}-name"),
        "class": "ws-mon",
        "bar": [ { "type": "label", "label": mon } ]
    })
}

/// The headless bar's own private pill — leads the strip, before any
/// physical monitor's pills. No ironvar and no number: the headless
/// output's own tags are already private (nobody else can see them), so
/// this doesn't need to own a specific one to mean "my own view" — it just
/// needs a fixed, always-first target that sends any pulled tag back,
/// which `remote.sh --restore` already does. The label is a static home
/// glyph, reusing [`overview_label`]'s three-line shape (invisible mirror
/// row / content / invisible mirror row) so its height matches a numbered
/// pill's exactly.
fn headless_pill() -> Value {
    let glyph = "\u{f02dc}"; // mdi-home
    let mirror = format!("<span size=\"38%\" alpha=\"1%\">{glyph}</span>");
    json!({
        "type": "custom",
        "name": "ws-home",
        "class": "ws ws-home",
        "bar": [ { "type": "button", "widgets": [
            { "type": "label", "justify": "center", "label": format!("{mirror}\n{glyph}\n{mirror}") }
        ] } ],
        "tooltip": "Your own view — click to send any pulled tag back",
        "on_click_left": "~/.config/ironbar/scripts/remote.sh --restore",
        "transition_type": "crossfade"
    })
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
        // T23: the tag pills live in `start` now, between spark and win —
        // see `start_modules()`'s own doc comment. Find the first tag pill
        // by its known name rather than assuming an index.
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let ws1 = start
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
        // T-hover: see volume_hover_targets_its_own_bar_name_and_drops_the_
        // click_toggle — same shape.
        // T15: on_click_left is powermode toggle now, moved from middle;
        // right stays untouched.
        // T23: `battery` is part of `resource_modules()`, which lives in
        // `end` now — see that function's own doc comment.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let battery = end.iter().find(|m| m["name"] == "battery").unwrap();
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
        let fb_end = fallback["end"].as_array().unwrap();
        let fb_battery = fb_end.iter().find(|m| m["name"] == "battery").unwrap();
        assert!(fb_battery["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-default"));
    }

    #[test]
    fn remote_left_toggles_vnc_right_toggles_kdeconnect() {
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let remote = end.iter().find(|m| m["name"] == "remote").unwrap();
        assert_eq!(
            remote["on_click_left"],
            json!("~/.config/ironbar/scripts/remote.sh --toggle-vnc")
        );
        assert_eq!(
            remote["on_click_right"],
            json!("~/.config/ironbar/scripts/remote.sh --toggle-kdeconnect")
        );
    }

    #[test]
    fn remote_popup_is_the_plain_read_only_stack() {
        // Replaces `remote_popup_has_one_pull_row_per_physical_monitor…`.
        // T-remote-popup deleted the clickable tag grid from the physical
        // bars' `remote` pill. Equality against `popup()` is the whole
        // assertion for this bar: the plain stack has no extra section by
        // construction, so no grid row and no button can hide in it.
        //
        // T-headless-strip put `--pull` back — deliberately, on the
        // HEADLESS bar only, where a remote viewer can actually reach it
        // (see genconfig.rs module doc / IRONBAR.md's T-headless-strip
        // entry). So the "no pull command anywhere" check narrows to the
        // physical bars and the fallback bar: nothing reachable only via
        // SUPER+CTRL keybinds (unreachable to a remote viewer) may also
        // carry a pull button.
        let cfg = build(&[
            "eDP-1".to_string(),
            "DP-1".to_string(),
            "HEADLESS-4".to_string(),
        ]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let remote = end.iter().find(|m| m["name"] == "remote").unwrap();
        assert_eq!(remote["popup"], popup("remote_tip", "bar-eDP-1"));
        for bar in ["eDP-1", "DP-1"] {
            let whole = serde_json::to_string(&cfg["monitors"][bar]).unwrap();
            assert!(
                !whole.contains("remote.sh --pull"),
                "no pull command may survive on {bar}'s own bar — a remote viewer can't reach it while the panels are dark"
            );
        }
        let fallback = build(&[]);
        assert!(!serde_json::to_string(&fallback).unwrap().contains("remote.sh --pull"));

        let edp_start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let edp_names: Vec<&str> = edp_start
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(edp_names.first(), Some(&"spark"));
        assert_eq!(edp_names.last(), Some(&"win"));
        assert!(cfg["monitors"]["eDP-1"]["center"].as_array().is_some());
        assert!(cfg["monitors"]["eDP-1"]["end"].as_array().is_some());
    }

    #[test]
    fn headless_bar_is_the_remote_control_strip() {
        // T-headless-strip: the HEADLESS bar's `start` is a private pill,
        // then one screen-name label + nine pull pills per physical
        // monitor — not a copy of the headless output's own tags (that was
        // T-headless-bar; see the module doc comment above `workspace_pills`
        // for why it was wrong).
        let cfg = build(&[
            "eDP-1".to_string(),
            "DP-1".to_string(),
            "HEADLESS-4".to_string(),
        ]);
        let headless = &cfg["monitors"]["HEADLESS-4"];
        assert!(headless.get("center").is_none());
        assert!(headless.get("end").is_none());

        let start = headless["start"].as_array().unwrap();
        let names: Vec<String> = start
            .iter()
            .map(|m| m["name"].as_str().unwrap().to_string())
            .collect();
        let mut expected = vec!["ws-home".to_string(), "ws-eDP-1-name".to_string()];
        expected.extend((1..=TAG_COUNT).map(|n| ws_module("eDP-1", n)));
        expected.push("ws-DP-1-name".to_string());
        expected.extend((1..=TAG_COUNT).map(|n| ws_module("DP-1", n)));
        assert_eq!(names, expected);

        let home = &start[0];
        assert_eq!(home["on_click_left"], json!("~/.config/ironbar/scripts/remote.sh --restore"));

        for pill in start.iter().filter(|m| {
            let n = m["name"].as_str().unwrap();
            n.starts_with("ws-eDP-1-") && n != "ws-eDP-1-name" || n.starts_with("ws-DP-1-") && n != "ws-DP-1-name"
        }) {
            let name = pill["name"].as_str().unwrap();
            let (mon, tag) = name.strip_prefix("ws-").unwrap().rsplit_once('-').unwrap();
            assert_eq!(
                pill["on_click_left"],
                json!(format!("~/.config/ironbar/scripts/remote.sh --pull {mon} {tag}"))
            );
            assert!(pill.get("show_if").is_none(), "{name} must not be show_if-gated — see genconfig.rs's remote_pills doc comment");
            assert!(pill.get("on_scroll_up").is_none(), "{name} must not scroll the viewer's own headless view");
            assert!(pill.get("on_scroll_down").is_none());
        }

        assert!(!names.iter().any(|n| n.ends_with("-ov")), "no overview pill on the remote strip");
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
    fn start_holds_only_spark_tags_and_win_no_ambient_modules() {
        // T23: `start` is launcher -> tags -> focus only (statusbar-layout.md
        // §3.1) — the resource-monitor group moved to `end` (see
        // `resource_modules()`'s own doc comment), reversing T-next (item
        // 3)'s move of that group INTO `start`. Regression guard: none of
        // them may still be here.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let names: Vec<&str> = start.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(names.first(), Some(&"spark"));
        assert_eq!(names.last(), Some(&"win"));
        for name in ["sysload", "devload", "battery", "music"] {
            assert!(
                !names.contains(&name),
                "{name} must not still be in start"
            );
        }
    }

    #[test]
    fn sysload_battery_devload_sit_together_in_end() {
        // T23: the resource block (statusbar-layout.md §3.4) — general to
        // specific: cpu+memory -> battery -> claude+docker(+archupdate).
        // T29 merged cpu/memory into `sysload` and claudebar/docker/
        // archupdate into `devload` — see `resource_modules()`'s own doc
        // comment. `music` is deliberately absent: it moved to
        // `audio_modules()` instead (see that function's own doc comment)
        // — grouped with volume/mic by domain, not kept alongside the
        // machine-health pills.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let tray = names.iter().position(|n| *n == "tray").unwrap();
        let sysload = names.iter().position(|n| *n == "sysload").unwrap();
        let battery = names.iter().position(|n| *n == "battery").unwrap();
        let devload = names.iter().position(|n| *n == "devload").unwrap();
        assert!(tray < sysload, "resource block must follow tray in end");
        assert!(sysload < battery && battery < devload);
    }

    #[test]
    fn all_36_modules_survive_the_reorder() {
        // Conservation check (statusbar-layout.md's own acceptance
        // criterion): a reorder must never silently drop a pill. Every name
        // present in the OLD start/center/end shape (T22, before T23) must
        // still be present somewhere in the new one, and the total count
        // must stay 36 for a two-monitor build (9 tags + 1 overview, times
        // 2 monitors, plus the 16 bar-global/static modules).
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let mut names = HashSet::new();
        collect_module_names(&json!({"start": start, "center": center, "end": end}), &mut names);
        let expected = [
            "spark", "win", "clock", "date", "pomo", "tray", "sysload", "battery", "devload",
            "colorpicker", "darkmode", "snip", "inhibit", "music", "volume",
            "mic", "net-spinner", "wifi", "eth", "netsec", "hotspot", "remote", "bluetooth",
            "power",
            // T28: the pill promoted out of the tray drawer it gates
            // (T29: `archupdate` is no longer one of these — it folded
            // into `devload`'s docker row). `traytoggle` itself is gone
            // from this list — the drawer is disabled by default now
            // (`TRAY_DRAWER_ENABLED`), so its trigger is never generated.
            "keepass",
        ];
        for name in expected {
            assert!(names.contains(name), "{name} missing after reorder");
        }
        for tag in 1..=TAG_COUNT {
            assert!(names.contains(&ws_module("eDP-1", tag)));
        }
        assert!(names.contains(&ws_module_ov("eDP-1")));
        // T31: 25 named non-tag modules (26 post-T29 minus the `tools`
        // drawer module itself; its three nested tools survive as
        // standalone pills) + 9 tags + 1 overview = 35 names.
        assert_eq!(names.len(), 35);
    }

    // T23: `end_no_longer_carries_the_leftcenter_modules` (a T8d regression
    // guard against cpu/memory/docker/battery/claudebar/music living in
    // `end`) removed outright, not inverted — its own premise is now
    // backwards by design (`resource_modules()`/`audio_modules()` put them
    // back in `end` on purpose). Superseded by
    // `cpu_memory_docker_battery_claudebar_sit_together_in_end` (positive:
    // they ARE in `end`, in order) and
    // `start_holds_only_spark_tags_and_win_no_ambient_modules` (negative:
    // they are NOT in `start`) — together a strict superset of what this
    // test checked, so nothing is lost by removing it.

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
    fn devload_hover_targets_its_own_bar_name_and_nested_clicks_survive() {
        // T29: `claudebar`+`docker`(+`archupdate`) merged into `devload`
        // (see `devload_module()`'s own doc comment); each row/cell keeps
        // its own click as a real nested `WidgetConfig` field. T31: the
        // hover enter/exit pair sits on the outer module AND on every
        // nested button — the buttons' own GDK event windows swallow the
        // crossings the module-level handlers were waiting for (see
        // `devload_module()`'s own doc comment).
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let devload = end.iter().find(|m| m["name"] == "devload").unwrap();
        assert!(devload.get("on_click_left").is_none());
        assert!(devload["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert!(devload["on_mouse_exit"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));

        let rows = devload["bar"][0]["widgets"].as_array().unwrap();
        let claude_row = &rows[0];
        assert_eq!(claude_row["class"], json!("row-claude"));
        assert!(claude_row.get("on_click_left").is_none());
        assert_eq!(
            claude_row["on_click_right"],
            json!("xdg-open https://claude.ai/settings/usage")
        );

        let svc_cells = rows[1]["widgets"].as_array().unwrap();
        for button in [claude_row]
            .into_iter()
            .chain(svc_cells.iter().filter(|c| c["type"] == "button"))
        {
            assert_eq!(button["on_mouse_enter"], devload["on_mouse_enter"]);
            assert_eq!(button["on_mouse_exit"], devload["on_mouse_exit"]);
        }
        let docker_cell = svc_cells.iter().find(|c| c["class"] == "cell-docker").unwrap();
        assert_eq!(
            docker_cell["on_click_right"],
            json!("~/.config/ironbar/scripts/docker-menu.sh")
        );
        let au_cell = svc_cells.iter().find(|c| c["class"] == "cell-au").unwrap();
        assert_eq!(au_cell["show_if"], json!("#au_show"));
        assert_eq!(
            au_cell["on_click_left"],
            json!("kitty --class mango-monitor -e arch-update")
        );

        let fallback = build(&[]);
        let fb_end = fallback["end"].as_array().unwrap();
        let fb_devload = fb_end.iter().find(|m| m["name"] == "devload").unwrap();
        assert!(fb_devload["on_mouse_enter"]
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
    fn tools_are_standalone_pills_in_reading_order_before_inhibit() {
        // T31: the tools drawer is gone (see `tools_modules()`'s own doc
        // comment) — colorpicker/darkmode/snip are standalone modules
        // again, always visible (no `show_if`), in the T23 reading order,
        // with `inhibit` right after. Real per-monitor bar and the
        // fallback bar both get them.
        let cfg = build(&["eDP-1".to_string(), "DP-1".to_string()]);
        for mon in ["eDP-1", "DP-1"] {
            let end = cfg["monitors"][mon]["end"].as_array().unwrap();
            let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
            let cp = names.iter().position(|n| *n == "colorpicker").unwrap();
            let dm = names.iter().position(|n| *n == "darkmode").unwrap();
            let sn = names.iter().position(|n| *n == "snip").unwrap();
            let inh = names.iter().position(|n| *n == "inhibit").unwrap();
            assert!(cp < dm && dm < sn && sn < inh);
            assert_eq!(sn + 1, inh);
            for name in ["colorpicker", "darkmode", "snip"] {
                let m = end.iter().find(|m| m["name"] == name).unwrap();
                assert!(m.get("show_if").is_none(), "{name} must be always visible");
                assert!(m["tooltip"].is_string(), "{name} needs its static tooltip");
                assert!(m["on_click_left"].is_string());
            }
            let darkmode = &end[dm];
            assert_eq!(darkmode["bar"][0]["type"], json!("button"));
            assert_eq!(
                darkmode["on_click_left"],
                json!("~/.config/ironbar/scripts/darkmode.sh --toggle")
            );
        }
        let fallback = build(&[]);
        let fb_end = fallback["end"].as_array().unwrap();
        for name in ["colorpicker", "darkmode", "snip"] {
            assert!(fb_end.iter().any(|m| m["name"] == name));
        }
    }

    // ---- T7a additions

    #[test]
    fn sysload_hover_targets_its_own_bar_name_and_rows_keep_their_click() {
        // T29: `cpu`/`memory` merged into one `sysload` module (see
        // `sysload_module()`'s own doc comment); main.rs's hover state
        // machine runs both cpu-detail/mem-detail refreshes before
        // opening one popup. T31: the two-row stack became one single-row
        // `button` bar root — hover and click live on that button, and
        // the two gauges/labels carry per-resource classes so the level
        // and warning CSS can tell them apart under the one root.
        // T15: btop moved from middle- to left-click.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let sysload = end.iter().find(|m| m["name"] == "sysload").unwrap();
        assert!(sysload["on_mouse_enter"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert!(sysload["on_mouse_exit"]
            .as_str()
            .unwrap()
            .contains("bar-eDP-1"));
        assert!(sysload.get("on_click_left").is_none());

        let root = &sysload["bar"][0];
        assert_eq!(root["type"], json!("button"));
        assert_eq!(
            root["on_click_left"],
            json!("kitty --class mango-monitor -e btop")
        );
        assert!(root.get("on_click_middle").is_none());
        // T31 follow-up: one vertical stack of two windowless rows under
        // the root button — no nested `button` anywhere (a nested
        // button's event window would swallow the root's hover again).
        let stack = &root["widgets"][0];
        assert_eq!(stack["orientation"], json!("vertical"));
        let rows = stack["widgets"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        for (row, (ico, gauge)) in rows
            .iter()
            .zip([("cpu-ico", "gauge-cpu"), ("mem-ico", "gauge-mem")])
        {
            assert_eq!(row["type"], json!("box"));
            let w = row["widgets"].as_array().unwrap();
            assert_eq!(w[0]["type"], json!("label"));
            assert_eq!(w[0]["class"], json!(ico));
            assert_eq!(w[1]["type"], json!("box"));
            assert_eq!(w[1]["class"], json!(gauge));
        }

        // T-popup-vert: popup is one outer vertical box (see popup()'s own
        // doc comment); `popup_multi` gives each of the two sections its
        // own title/sep/body/sep/hint (5 widgets), plus one leading
        // separator between them (T29) — 11 total, no poke widget inside
        // either section the S2 finding ruled out.
        let popup = sysload["popup"].as_array().unwrap();
        assert_eq!(popup.len(), 1);
        assert_eq!(popup[0]["widgets"].as_array().unwrap().len(), 11);

        let fallback = build(&[]);
        let fb_end = fallback["end"].as_array().unwrap();
        let fb_sysload = fb_end.iter().find(|m| m["name"] == "sysload").unwrap();
        assert!(fb_sysload["on_mouse_enter"]
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
        // T23: `clock`/`date` are `time_modules()`'s own content, which
        // lives in `center` on every bar now (this reverses T-next item 3's
        // move into `end` — see `time_modules()`'s own doc comment).
        let cfg = build(&["eDP-1".to_string()]);
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let clock = center.iter().find(|m| m["name"] == "clock").unwrap();
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
            let module = center.iter().find(|m| m["name"] == name).unwrap();
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
        // T23: `date` is `time_modules()`'s own content, in `center` on
        // every bar now.
        let cfg = build(&["eDP-1".to_string()]);
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let date = center.iter().find(|m| m["name"] == "date").unwrap();
        assert_eq!(
            date["on_click_left"],
            json!("~/.config/ironbar/scripts/clock.sh --calendar")
        );
        assert!(date.get("on_click_middle").is_none());
    }

    // ---- T8a additions: tray, bluetooth, music, inhibit, static buttons.

    #[test]
    fn tray_leads_end_and_the_resource_block_follows() {
        // T23/T31: `end` is now tray -> resources -> darkmode/inhibit ->
        // audio -> connectivity -> session (statusbar-layout.md §3.3-3.8,
        // tools drawer removed in T31) — see
        // `end_modules()`'s own doc comment. Tray leads the WHOLE row now,
        // not just its own sub-group, since `center` carries the time
        // block instead of `end` borrowing it.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        assert!(!start.iter().any(|m| m["name"] == "tray"));
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(names.first(), Some(&"tray"));
        let sysload = names.iter().position(|n| *n == "sysload").unwrap();
        let volume = names.iter().position(|n| *n == "volume").unwrap();
        assert!(sysload < volume, "resources must precede audio in end");
    }

    #[test]
    fn spark_leads_start_tags_follow_win_is_last() {
        // T23: the tag pills moved back into `start`, between `spark` and
        // `win` (statusbar-layout.md §3.1 — reverses T-next item 3's move
        // into `center`; see `start_modules()`'s own doc comment). `win`
        // stays last: it is the one unbounded-width field, INV-1's growth
        // end.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let names: Vec<&str> = start.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(names.first(), Some(&"spark"));
        assert_eq!(names.last(), Some(&"win"));
        let spark = names.iter().position(|n| *n == "spark").unwrap();
        let ws1 = names
            .iter()
            .position(|n| *n == ws_module("eDP-1", 1))
            .unwrap();
        let win = names.iter().position(|n| *n == "win").unwrap();
        assert!(spark < ws1 && ws1 < win);

        let fallback = build(&[]);
        let fb_start = fallback["start"].as_array().unwrap();
        assert!(fb_start.iter().any(|m| m["name"] == "spark"));
        assert!(
            !fb_start.iter().any(|m| m["name"] == ws_module("eDP-1", 1)),
            "fallback bar has no monitor slug to build tag pills from"
        );
    }

    #[test]
    fn bluetooth_and_power_land_at_the_end_of_end() {
        // T23: connectivity -> session is still the tail of `end`
        // (statusbar-layout.md §3.7-3.8) — `music` lives in `audio_modules()`
        // now (see that function's own doc comment), so it's absent from
        // this specific trailing check.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        let names: Vec<&str> = end.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let hotspot = names.iter().position(|n| *n == "hotspot").unwrap();
        let bluetooth = names.iter().position(|n| *n == "bluetooth").unwrap();
        let power = names.iter().position(|n| *n == "power").unwrap();
        assert!(hotspot < bluetooth && bluetooth < power);
        // power is the last module of the last row — INV-3's corner target.
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
    fn tools_drawer_and_its_reveal_var_are_gone() {
        // T31: the tools drawer is removed (see `tools_modules()`'s own
        // doc comment). The `tools` module must not exist anywhere — top
        // level or nested — and the `tools_open` ironvar default dies
        // with it. The three tools themselves survive as standalone
        // pills (checked by
        // `tools_are_standalone_pills_in_reading_order_before_inhibit`).
        let cfg = build(&["eDP-1".to_string()]);
        let mut names = HashSet::new();
        collect_module_names(&cfg, &mut names);
        assert!(!names.contains("tools"), "the drawer module must be gone");
        assert!(cfg["ironvar_defaults"].get("tools_open").is_none());
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
        // T28: `music` dropped out of this group — it is a `custom` module
        // now, fed by music.rs (see `music_module()`'s own doc comment for
        // why: the native module's `set_label_escaped` render path can't
        // carry the two-line markup the window pill uses). `bluetooth` and
        // `tray` are still the real native modules on this bar.
        let cfg = build(&["eDP-1".to_string()]);
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        for name in ["bluetooth", "tray"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
            assert_eq!(m["type"], json!(name));
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
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();

        let win = start.iter().find(|m| m["name"] == "win").unwrap();
        assert!(win["tooltip"].is_string());
        // T19: wifi/eth/netsec moved out of this group — see below.
        let mic = end.iter().find(|m| m["name"] == "mic").unwrap();
        assert!(mic["tooltip"].is_string(), "mic missing a tooltip");

        // T23: all three are part of `resource_modules()`, which lives in
        // `end` now — see that function's own doc comment (T29: `cpu`/
        // `memory`/`docker`/`claudebar` are `sysload`/`devload` now). T28:
        // `music` joins this group — it lost its static tooltip the same
        // way these did, when it converted from a native module to a
        // hover-popup `custom` one (`music_module()`'s own doc comment).
        for name in ["sysload", "devload", "battery", "music"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
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
        // T23: tags live in `start` now — see `start_modules()`'s own doc
        // comment.
        let ws1 = start
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
        // T23: cpu/memory/docker/battery/claudebar are part of
        // `resource_modules()` (now in `end`, and now `sysload`/`battery`/
        // `devload` after T29 — see that function's own doc comment);
        // clock/date/pomo are part of `time_modules()` (now in `center` on
        // every bar) — see both functions' own doc comments.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        // T28: `music`/`keepass` join this group — same hover-opens-the-
        // popup shape as the resource pills. T29: `archupdate` drops out
        // of this list — it is no longer a top-level module (folded into
        // `devload`'s docker row), so it has no `on_mouse_enter` of its
        // own to check here any more.
        for name in ["sysload", "devload", "battery", "music", "keepass"] {
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
        for name in ["clock", "date", "pomo"] {
            let m = center.iter().find(|m| m["name"] == name).unwrap();
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
        // T23: tags live in `start` now.
        for n in 1..=TAG_COUNT {
            let module = ws_module("eDP-1", n);
            let m = start.iter().find(|m| m["name"] == module).unwrap();
            assert!(
                m["on_mouse_enter"].is_string(),
                "{module} missing on_mouse_enter"
            );
            assert!(
                m["on_mouse_exit"].is_string(),
                "{module} missing on_mouse_exit"
            );
        }
        // T31: the tools drawer's own hover-toggle check is gone with the
        // drawer (see `tools_modules()`'s own doc comment).
    }

    #[test]
    fn non_popup_modules_carry_no_mouse_attributes() {
        // A pill with no `popup` field has nothing to show — hover there
        // must stay a no-op, so these must carry neither attribute.
        //
        // T31: `colorpicker`/`darkmode`/`snip` rejoin this list — they
        // are standalone modules again (the tools drawer is gone, see
        // `tools_modules()`'s own doc comment), popup-free with static
        // tooltips, so hover must stay a no-op on each.
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
            "mic",
            "net-spinner",
            "colorpicker",
            "darkmode",
            "snip",
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
        // T23: `pomo` is `time_modules()`'s own content, in `center` on
        // every bar now.
        let cfg = build(&["eDP-1".to_string()]);
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let pomo = center.iter().find(|m| m["name"] == "pomo").unwrap();
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
        // T23: cpu/memory/docker/battery/claudebar are part of
        // `resource_modules()` (now in `end`, now `sysload`/`battery`/
        // `devload` after T29 — see that function's own doc comment);
        // clock/date are part of `time_modules()` (now in `center` on
        // every bar).
        let cfg = build(&["eDP-1".to_string()]);
        let center = cfg["monitors"]["eDP-1"]["center"].as_array().unwrap();
        let end = cfg["monitors"]["eDP-1"]["end"].as_array().unwrap();
        for name in ["sysload", "devload", "battery"] {
            let m = end.iter().find(|m| m["name"] == name).unwrap();
            if let Some(click) = m.get("on_click_left").and_then(|v| v.as_str()) {
                assert!(
                    !click.contains("toggle-popup"),
                    "{name}'s on_click_left must not send toggle-popup: {click}"
                );
            }
        }
        // T29: nested clicks get the same guard, one level down. T31:
        // `sysload`'s click moved to its single bar-root button; the
        // module-level loop above does not see widget-level keys, so pin
        // the root button separately.
        {
            let sysload = end.iter().find(|m| m["name"] == "sysload").unwrap();
            let click = sysload["bar"][0]["on_click_left"].as_str().unwrap();
            assert!(
                !click.contains("toggle-popup"),
                "sysload root button's on_click_left must not send toggle-popup: {click}"
            );
        }
        for (module, class) in [("devload", "row-claude")] {
            let m = end.iter().find(|m| m["name"] == module).unwrap();
            let rows = m["bar"][0]["widgets"].as_array().unwrap();
            let row = rows.iter().find(|r| r["class"] == class).unwrap();
            if let Some(click) = row.get("on_click_left").and_then(|v| v.as_str()) {
                assert!(
                    !click.contains("toggle-popup"),
                    "{module}.{class}'s on_click_left must not send toggle-popup: {click}"
                );
            }
        }
        {
            let devload = end.iter().find(|m| m["name"] == "devload").unwrap();
            let svc_cells = devload["bar"][0]["widgets"][1]["widgets"].as_array().unwrap();
            for cell in svc_cells {
                if let Some(click) = cell.get("on_click_left").and_then(|v| v.as_str()) {
                    assert!(
                        !click.contains("toggle-popup"),
                        "devload.{}'s on_click_left must not send toggle-popup: {click}",
                        cell["class"]
                    );
                }
            }
        }
        for name in ["clock", "date"] {
            let m = center.iter().find(|m| m["name"] == name).unwrap();
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
        // bluetooth/clock carry no on_click_left at all, and neither do
        // devload's claude row or docker cell (the au cell is the one
        // exception — it runs arch-update, checked separately) — still
        // true, and worth pinning so a future edit that adds one notices
        // this test instead of sliding past it silently.
        {
            let devload = end.iter().find(|m| m["name"] == "devload").unwrap();
            assert!(devload.get("on_click_left").is_none());
            let rows = devload["bar"][0]["widgets"].as_array().unwrap();
            let claude_row = rows.iter().find(|r| r["class"] == "row-claude").unwrap();
            assert!(claude_row.get("on_click_left").is_none());
            let svc_cells = rows[1]["widgets"].as_array().unwrap();
            let docker_cell = svc_cells.iter().find(|c| c["class"] == "cell-docker").unwrap();
            assert!(docker_cell.get("on_click_left").is_none());
        }
        let clock = center.iter().find(|m| m["name"] == "clock").unwrap();
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
        // T23: tags live in `start` now.
        let cfg = build(&["eDP-1".to_string()]);
        let start = cfg["monitors"]["eDP-1"]["start"].as_array().unwrap();
        let ws1 = start
            .iter()
            .find(|m| m["name"] == ws_module("eDP-1", 1))
            .unwrap();
        let click = ws1["on_click_right"].as_str().unwrap();
        assert!(click.contains("toggle-popup"));
        assert!(click.contains("bar-eDP-1"));
    }
}
