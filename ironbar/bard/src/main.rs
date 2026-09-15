//! mango-bard — event-driven daemon feeding ironbar's ironvars. T1 scope:
//! clock + powermode watcher + control socket + ironbar IPC client. See
//! /home/martin/work/mango-dotfiles/IRONBAR.md for the full design and the
//! T0 spike findings this implementation is built on.

mod archupdate;
mod audio;
mod claude;
mod clock;
mod cmd;
mod control;
mod cpu;
mod darkmode;
mod docker;
mod genconfig;
mod hotspot;
mod ipc;
mod keepass;
mod keepawake;
mod mango;
mod memory;
mod music;
mod net;
mod pomo;
mod power;
mod powermode;
mod remote;
mod routes;
mod sys;
mod tooltip;
mod vars;
mod wheel;

use archupdate::Archupdate;
use audio::Audio;
use claude::Claude;
use clock::Clock;
use control::{HoverEvent, Line, PopupHoverEvent};
use cpu::Cpu;
use darkmode::Darkmode;
use docker::Docker;
use hotspot::Hotspot;
use ipc::IronbarIpc;
use keepass::Keepass;
use keepawake::Keepawake;
use mango::Mango;
use memory::Memory;
use music::Music;
use net::Net;
use pomo::Pomo;
use power::Power;
use powermode::PowermodeWatch;
use remote::Remote;
use std::time::{Duration, Instant};
use tokio::io::unix::AsyncFd;
use vars::Vars;
use wheel::Wheel;

const FLUSH_DEBOUNCE: Duration = Duration::from_millis(150);

/// How long the mouse must stay over a hover-eligible pill before its popup
/// opens. A calibration knob, same convention as net.rs's own debounce —
/// tuned, not derived. T13: dropped from 300ms to 80ms once
/// `ECO_TIMER_SLACK_MS` was found to be the real ~650-850ms cause (see
/// IRONBAR.md's T13 entry) — 80ms matches the value this repo already chose
/// for the same first-hover question in `waybar/fast-tooltips.c`
/// (`MANGO_TOOLTIP_DELAY_MS` default): short enough to read as instant, long
/// enough that a mouse passing through on its way elsewhere never triggers a
/// popup at all. `waybar/fast-tooltips.c` moved to `ironbar/fast-tooltips.c`
/// at the T8 cutover; the constant it tunes and this value are unchanged.
const HOVER_DELAY: Duration = Duration::from_millis(80);

/// T-popup-hold: how long a popup stays visible after the pointer leaves
/// its pill, before actually closing — long enough to cross the `popup_gap`
/// (12px, see genconfig.rs's own comment on that setting) into the popup
/// itself, short enough that a popup abandoned for good doesn't linger. A
/// calibration knob, same convention as `HOVER_DELAY` above — tuned against
/// a live screenshot/pointer test, not derived. Cancelled outright by
/// `hover_enter` (re-hovering any pill) or `hover_hold` (the pointer
/// reaching the popup's own content box).
const HOVER_HIDE_GRACE: Duration = Duration::from_millis(200);

/// `sys::set_timer_slack` value while genuinely idle (no hover pending, no
/// dirty ironvars waiting to flush) — the existing eco knob, unchanged in
/// magnitude. T13 scoped it to idle-only for `hover_due`, because 500ms of
/// slack was silently adding up to 500ms to every `hover_due` firing on top
/// of `HOVER_DELAY` itself; a later pass found `dirty_since` (the workspace
/// pill flush debounce) paid the same tax and folded both into one
/// recompute at the top of the event loop — see that call site.
const ECO_TIMER_SLACK_MS: u64 = 500;

struct Stats {
    wakeups: u64,
    var_sets: u64,
    flushes: u64,
    ipc_errors: u64,
    /// Lines received from either `mmsg watch` child, before dedup.
    mmsg_events: u64,
    /// Of those, how many were byte-identical to the previous document for
    /// that topic (eco-invariant dedup layer 1 — see mango.rs).
    mmsg_noop: u64,
    /// Lines received from either net.rs `MonitorChild` (`nmcli monitor`,
    /// `ip -o monitor route`), before dedup — same shape as `mmsg_events`.
    net_events: u64,
    /// Of those, how many were byte-identical to the previous line on that
    /// child (net.rs's `ingest_*` dedup layer 1, mirroring mango.rs).
    net_noop: u64,
    /// Successful `style add-class`/`remove-class` round trips.
    style_sets: u64,
    /// Lines received from `audio.rs`'s `pactl subscribe` child, before
    /// dedup — same shape as `net_events`.
    audio_events: u64,
    /// Of those, how many were either byte-identical to the previous line
    /// or classified as client noise (`event_matches()` — volume.sh:73).
    audio_noop: u64,
    /// Lines received from `power.rs`'s `udevadm monitor` child. No dedup
    /// counter: `udevadm monitor` timestamps every line, so byte-dedup can
    /// never fire — every line is regrade-worthy (mirrors ac-watch.sh
    /// reading "event count, not content").
    power_events: u64,
    /// Wheel fd wakeups (full/battery only — disarmed in eco, see wheel.rs).
    wheel_ticks: u64,
    cpu_polls: u64,
    mem_polls: u64,
    /// Lines received from `docker.rs`'s `docker events` child, before
    /// dedup — same shape as `audio_events`. hotspot.rs/darkmode.rs have no
    /// event stream of their own (T6b decisions D2/D3 — click-poke + a
    /// tick-gated backstop instead), so neither needs a counter here.
    docker_events: u64,
    /// Of those, how many were byte-identical to the previous line (D1: the
    /// server-side `--filter` set is docker.rs's real noise gate, so this
    /// should stay near zero).
    docker_noop: u64,
    /// T7a: how many times `cpu_tip`/`mem_tip` were actually rebuilt — the
    /// counters T7a's own verification measures against a closed popup,
    /// since `strace` is blocked by ptrace permissions in this environment
    /// (T4's own prior finding). Both must stay flat while both popups are
    /// closed; this is the ptrace-free proof of the "popup-closed strace
    /// shows zero execs" acceptance line.
    cpu_detail_builds: u64,
    mem_detail_builds: u64,
    /// T6c: how many times the claudebar cache was read + rendered — the
    /// cadence proof that this only happens once per 5-minute clock tick,
    /// same method T7a's `cpu_detail_builds` uses for its own cadence.
    claude_fetches: u64,
    /// T7c-rest: how many times `clk_tip`/`date_tip` were actually rebuilt
    /// — same acceptance shape as `cpu_detail_builds`: both must stay flat
    /// across a clean idle window and only climb on a real popup click,
    /// since `strace` is ptrace-blocked in this environment (T4 onwards).
    clock_detail_builds: u64,
    date_detail_builds: u64,
    /// T19: how many times `wifi_tip`/`eth_tip`/`sec_tip` were actually
    /// rebuilt — same acceptance shape as `cpu_detail_builds`/
    /// `clock_detail_builds`: flat while every net popup is closed, climbs
    /// only on a real hover-open.
    wifi_detail_builds: u64,
    eth_detail_builds: u64,
    sec_detail_builds: u64,
    started: Instant,
}

impl Stats {
    fn new() -> Self {
        Self {
            wakeups: 0,
            var_sets: 0,
            flushes: 0,
            ipc_errors: 0,
            mmsg_events: 0,
            mmsg_noop: 0,
            net_events: 0,
            net_noop: 0,
            style_sets: 0,
            audio_events: 0,
            audio_noop: 0,
            power_events: 0,
            wheel_ticks: 0,
            cpu_polls: 0,
            mem_polls: 0,
            docker_events: 0,
            docker_noop: 0,
            cpu_detail_builds: 0,
            mem_detail_builds: 0,
            claude_fetches: 0,
            clock_detail_builds: 0,
            date_detail_builds: 0,
            wifi_detail_builds: 0,
            eth_detail_builds: 0,
            sec_detail_builds: 0,
            started: Instant::now(),
        }
    }

    fn to_json(&self, mode: powermode::Mode) -> String {
        format!(
            "{{\"wakeups\":{},\"var_sets\":{},\"flushes\":{},\"ipc_errors\":{},\"mmsg_events\":{},\"mmsg_noop\":{},\"net_events\":{},\"net_noop\":{},\"style_sets\":{},\"audio_events\":{},\"audio_noop\":{},\"power_events\":{},\"wheel_ticks\":{},\"cpu_polls\":{},\"mem_polls\":{},\"docker_events\":{},\"docker_noop\":{},\"cpu_detail_builds\":{},\"mem_detail_builds\":{},\"claude_fetches\":{},\"clock_detail_builds\":{},\"date_detail_builds\":{},\"wifi_detail_builds\":{},\"eth_detail_builds\":{},\"sec_detail_builds\":{},\"uptime_s\":{},\"mode\":\"{}\"}}",
            self.wakeups,
            self.var_sets,
            self.flushes,
            self.ipc_errors,
            self.mmsg_events,
            self.mmsg_noop,
            self.net_events,
            self.net_noop,
            self.style_sets,
            self.audio_events,
            self.audio_noop,
            self.power_events,
            self.wheel_ticks,
            self.cpu_polls,
            self.mem_polls,
            self.docker_events,
            self.docker_noop,
            self.cpu_detail_builds,
            self.mem_detail_builds,
            self.claude_fetches,
            self.clock_detail_builds,
            self.date_detail_builds,
            self.wifi_detail_builds,
            self.eth_detail_builds,
            self.sec_detail_builds,
            self.started.elapsed().as_secs(),
            mode.as_str(),
        )
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("run") | None => run().await,
        Some("refresh") => match args.get(2) {
            // T7a: the popup's own lazy-poke script (genconfig.rs's
            // `cpu_modules`) calls this every few seconds while open and
            // discards nothing itself (ironbar execs scripts directly, no
            // shell — confirmed live — so a `>/dev/null` redirect would be
            // passed through as a literal argv token, not interpreted).
            // `-q` is what keeps the daemon's `ok` reply out of the popup's
            // otherwise-invisible poke label.
            Some(topic) => {
                let quiet = args.iter().skip(3).any(|a| a == "-q");
                client_cmd(&format!("refresh {topic}"), quiet).await
            }
            None => Err("usage: mango-bard refresh <topic> [-q]".into()),
        },
        Some("ping") => client_cmd("ping", false).await,
        Some("stats") => client_cmd("stats", false).await,
        Some("gen-config") => genconfig::main(&args[2..]).await,
        // `mango-bard pomo start|toggle|reset|mute|note|resume|idle|unlock|status [arg] [-q]`
        // — one control-socket line, same shape as `refresh` above. `-q`
        // matters here too: focus-note.sh/focus-task.sh call this from a
        // rofi callback where a stray "ok" on stdout would be distracting.
        Some("pomo") => match args.get(2) {
            Some(verb) => {
                let rest: Vec<&str> = args.iter().skip(3).map(String::as_str).collect();
                let quiet = rest.contains(&"-q");
                let arg = rest.into_iter().find(|a| *a != "-q");
                let cmd = match arg {
                    Some(a) => format!("pomo {verb} {a}"),
                    None => format!("pomo {verb}"),
                };
                client_cmd(&cmd, quiet).await
            }
            None => Err("usage: mango-bard pomo <verb> [arg] [-q]".into()),
        },
        // `mango-bard hover enter|exit <bar> <widget> [-q]` — one
        // control-socket line, same shape as `pomo` above. Fired straight
        // from genconfig.rs's `on_mouse_enter`/`on_mouse_exit`, so `-q`
        // matters here too: ironbar has no visible surface for a stray "ok"
        // reply to land on, but staying quiet keeps this consistent with
        // every other fire-and-forget gesture in this file (T-hover).
        //
        // T-popup-hold: `hover hold|release <bar> [-q]` — the same verbs,
        // fired from the popup's own content box instead of a pill, so
        // there is no widget name to pass (see `control::PopupHoverEvent`'s
        // own doc comment).
        Some("hover") => {
            let rest: Vec<&str> = args.iter().skip(2).map(String::as_str).collect();
            let quiet = rest.contains(&"-q");
            let parts: Vec<&str> = rest.into_iter().filter(|a| *a != "-q").collect();
            match parts.as_slice() {
                [verb @ ("enter" | "exit"), bar, widget] => {
                    client_cmd(&format!("hover {verb} {bar} {widget}"), quiet).await
                }
                [verb @ ("hold" | "release"), bar] => {
                    client_cmd(&format!("hover {verb} {bar}"), quiet).await
                }
                _ => Err(
                    "usage: mango-bard hover <enter|exit> <bar> <widget> [-q] | hover <hold|release> <bar> [-q]"
                        .into(),
                ),
            }
        }
        _ => Err(
            "usage: mango-bard [run|refresh <topic> [-q]|ping|stats|gen-config|pomo <verb> [arg] [-q]|hover <enter|exit> <bar> <widget> [-q]|hover <hold|release> <bar> [-q]]"
                .into(),
        ),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// Drains every dirty ironvar over IPC — extracted verbatim from what used
/// to be the `dirty_since` select arm's own inline body (T13), so the hover
/// path can call the same flush synchronously before `show_popup` instead of
/// waiting out a separate `FLUSH_DEBOUNCE` after the popup is already open
/// (that gap showed stale tip content for the debounce's duration). Leaves
/// `dirty_since` itself untouched — that is loop state the caller owns, not
/// flush state.
async fn flush_vars(vars: &mut Vars, ipc: &mut IronbarIpc, stats: &mut Stats) {
    stats.flushes += 1;
    if ipc.restarted() {
        vars.mark_all_dirty();
    }
    loop {
        let next = vars
            .peek_dirty()
            .map(|(k, v)| (k.to_string(), v.to_string()));
        let Some((k, v)) = next else { break };
        // `@class/` keys never reach the wire as ironvars — they route to
        // `style add-class`/`remove-class` instead (mango.rs: CLASS_PREFIX
        // doc comment). A module may hold more than one independently
        // dirty-tracked class (netsec needs both its verdict class and an
        // independent `eco` class — IRONBAR.md T3): the key format is
        // `@class/<module>[#<slot>]`, and only the module half (before `#`)
        // is a real ironbar module name — the slot exists purely to keep
        // the two keys apart in `Vars`.
        let result = match k.strip_prefix(mango::CLASS_PREFIX) {
            Some(module_key) => {
                let module = module_key.split('#').next().unwrap_or(module_key);
                let old = vars.live_value(&k).map(str::to_string);
                let r = ipc.set_class(module, old.as_deref(), &v).await;
                if r.is_ok() {
                    stats.style_sets += 1;
                }
                r
            }
            None => ipc.var_set(&k, &v).await,
        };
        match result {
            Ok(()) => {
                vars.ack(&k);
                stats.var_sets += 1;
                if ipc.note_success() {
                    eprintln!("mango-bard: ipc recovered, sends succeeding again");
                }
            }
            Err(e) => {
                stats.ipc_errors += 1;
                // Log the down-edge only, not every failing key on every
                // flush — an ironbar outage used to log ~2000 lines/minute
                // here, one per dirty key per flush, for as long as ironbar
                // stayed down (T-keepass-loop).
                if ipc.note_failure() {
                    eprintln!("mango-bard: ipc error setting {k}: {e}");
                }
                // Cool this key down instead of leaving it as the permanent
                // head of `dirty` — otherwise a key that fails forever (e.g.
                // a workspace class for an output mango has destroyed) wins
                // `break` every flush and starves every sibling key behind
                // it in the map (T-freeze-2026-08-26).
                vars.back_off(&k);
            }
        }
    }
}

/// Per-topic refresh dispatch, shared by the click path (`Line::Refresh`,
/// `mango-bard refresh <topic> -q`) and the hover-open select arm — extracted
/// from what used to be `Line::Refresh`'s own inline match so the two
/// callers can never drift into two different topic tables (T-hover).
#[allow(clippy::too_many_arguments)]
async fn dispatch_refresh(
    topic: &str,
    vars: &mut Vars,
    mango: &mut Mango,
    net: &mut Net,
    audio: &mut Audio,
    power: &mut Power,
    cpu: &mut Cpu,
    mem: &mut Memory,
    docker: &mut Docker,
    hotspot: &mut Hotspot,
    remote: &mut Remote,
    keepawake: &mut Keepawake,
    darkmode: &mut Darkmode,
    claude: &mut Claude,
    keepass: &mut Keepass,
    archupdate: &Archupdate,
    pm_mode: powermode::Mode,
    stats: &mut Stats,
    regrade_due: &mut Option<Instant>,
    audio_due: &mut Option<Instant>,
    power_due: &mut Option<Instant>,
    docker_due: &mut Option<Instant>,
) {
    // switchwall.sh sends this after every matugen regen, ahead of the
    // full resync below — "colors" isn't its own match arm, so it falls
    // straight through to the `_` arm the same way an unrecognized topic
    // does, and darkmode.refresh() still runs from there.
    if topic == "colors" {
        crate::tooltip::reload_palette();
    }
    // remote.sh's --toggle-vnc pokes this after `ironbar reload` (a new/dropped
    // HEADLESS bar needs a fresh config). A plain reload swaps ironbar's
    // widget tree back to its process-start state — every ironvar it holds
    // resets — but this daemon's own `Vars::live` cache never learns that,
    // so the `_` catch-all's unconditional refresh below is a no-op for
    // anything whose value hasn't actually changed (`Vars::set` diffs
    // against `live` first). `mark_all_dirty()` clears that cache so the
    // same catch-all re-sends everything instead of trusting stale ACKs.
    // "resync" isn't its own match arm for the same reason "colors" isn't —
    // it needs the full catch-all to run right after.
    if topic == "resync" {
        vars.mark_all_dirty();
    }
    match topic {
        "clock" => clock::refresh(vars),
        "mango" | "workspaces" | "window" => mango.apply(vars),
        "net" | "netsec" | "wifi" | "eth" | "wifi-scan" => {
            net.refresh_busy_var(vars);
            *regrade_due = Some(Instant::now());
        }
        // T19: unlike the debounced "net"/"netsec"/"wifi"/"eth" arm above,
        // these three build their tip synchronously — only ever poked from
        // inside a hover-open (main.rs's hover select arm awaits this
        // directly, then flushes, then calls show_popup). Routing hover
        // through the debounced arm instead would show a stale popup: the
        // 150ms `regrade_due` debounce fires AFTER show_popup already ran,
        // the exact T13 bug the hover arm's own comment describes. Same
        // "lazily-built detail tip" shape as cpu-detail/mem-detail below.
        "wifi-detail" => {
            net.refresh_wifi_tip(vars).await;
            stats.wifi_detail_builds += 1;
        }
        "eth-detail" => {
            net.refresh_eth_tip(vars).await;
            stats.eth_detail_builds += 1;
        }
        "sec-detail" => {
            net.refresh_sec_tip(vars).await;
            stats.sec_detail_builds += 1;
        }
        "audio" | "volume" | "mic" => {
            *audio_due = Some(Instant::now());
        }
        "power" | "battery" | "bat" | "ac" => {
            *power_due = Some(Instant::now());
        }
        "cpu" => cpu.refresh(vars),
        "memory" | "mem" => mem.refresh(vars),
        // T7a: only ever poked from inside the popup's own script while it
        // is open (genconfig.rs's `cpu_modules`) or, since T-hover, the
        // hover-open select arm — never from the wheel/clock path, so
        // nothing here runs while neither the popup nor a hover-pending
        // peek is active.
        "cpu-detail" => {
            cpu.refresh_detail(vars).await;
            stats.cpu_detail_builds += 1;
        }
        "mem-detail" => {
            mem.refresh_detail(vars).await;
            stats.mem_detail_builds += 1;
        }
        // T7c-rest: only ever poked from inside the clock/date popup's own
        // click, or now its hover-open — same shape as cpu-detail/
        // mem-detail above.
        "clock-detail" => {
            clock::refresh_clock_tip(vars).await;
            stats.clock_detail_builds += 1;
        }
        "date-detail" => {
            clock::refresh_date_tip(vars).await;
            stats.date_detail_builds += 1;
        }
        "docker" => {
            *docker_due = Some(Instant::now());
        }
        // hotspot.sh's toggle() pokes this on the actual state-change edge
        // only (reached from --toggle/--menu, never --status) — T6b D2's
        // only event source for this collector.
        "hotspot" => hotspot.refresh(vars).await,
        // remote.sh's toggle verbs poke this on the actual state-change
        // edge — same shape as hotspot's arm above.
        "remote" => remote.refresh(vars).await,
        // keepawake.sh's toggle() pokes this on the actual state-change
        // edge — same shape as remote's arm above.
        "keepawake" => keepawake.refresh(vars).await,
        // T28: forces a resync right as the popup opens — cheap (two
        // small forks), same reasoning as hotspot/remote/keepawake above.
        "keepass" => keepass.refresh(vars).await,
        // T28: pure file read, no fork — safe to call unconditionally.
        "archupdate" => archupdate.refresh(vars),
        // switchwall.sh pokes this after every matugen regen — T6b D3.
        "darkmode" => darkmode.refresh(vars).await,
        // T6c: pure render (cache read only — never a fork), fired either by
        // spawn_fetch()'s own post-fetch poke or by the popup's left-click
        // (T7a's refresh-then-toggle-popup pattern) or hover-open.
        "claude" => {
            claude.refresh(vars);
            stats.claude_fetches += 1;
        }
        // Deliberately excludes cpu-detail/mem-detail/clock-detail/
        // date-detail: this is the generic "resync every pill" path (e.g.
        // after an unrecognized topic), and rebuilding any detail popup
        // unconditionally here would defeat T7a's whole point — only a real
        // popup open (click or hover) may ever trigger that work.
        _ => {
            clock::refresh(vars);
            mango.apply(vars);
            audio.refresh(vars).await;
            power.refresh(vars, pm_mode).await;
            cpu.refresh(vars);
            mem.refresh(vars);
            docker.refresh(vars).await;
            hotspot.refresh(vars).await;
            remote.refresh(vars).await;
            keepawake.refresh(vars).await;
            darkmode.refresh(vars).await;
            claude.refresh(vars);
            keepass.refresh(vars).await;
            archupdate.refresh(vars);
        }
    }
}

/// Maps a hover target's ironbar widget name to the `dispatch_refresh`
/// topics that keep its popup content current — a slice, not a single
/// topic, because T29 merged `cpu`/`memory` into one `sysload` module and
/// `claudebar`/`docker`/`archupdate` into one `devload` module (a nested
/// module's own class updates never reach ironbar's style IPC — see
/// `sysload_module()`'s own doc comment — so each stack has to be one
/// top-level module with one popup covering every row). Most other hover
/// widgets still share their module name as their one topic (`battery`,
/// `volume` — both just mark their own `_due` var, since their event
/// streams already keep them fresh); the modules whose popup is a
/// lazily-built detail tip (T7a/T7c-rest) use their own `-detail` topic
/// instead.
///
/// An empty slice means "skip `dispatch_refresh` entirely", not "fall
/// through to the generic full resync" — two cases: `bluetooth` is a native
/// module with no daemon-tracked ironvar at all (nothing here could
/// refresh), and a workspace tag pill's tip is already kept live by
/// `mango.apply()` off its own `mmsg watch` event stream (mango.rs), so it
/// never goes stale between hovers. Calling the fallback resync for either
/// would cost a real refresh cycle for no matching content.
///
/// T15: `hotspot` joins the `battery`/`volume` group — its own module name
/// doubles as its refresh topic (`main.rs:379`), same as those two, now
/// that hover opens its popup instead of a click. `inhibit` (the widget)
/// maps to the `keepawake` topic instead — see its own match arm below for
/// why the two names differ here.
///
/// T19: `wifi`/`eth`/`netsec` join the `sysload`/`clock`/`date` group —
/// each maps to its own `-detail` topic (`wifi-detail`/`eth-detail`/
/// `sec-detail`), not its own module name, for the same reason those do:
/// the popup content is lazily built, not already kept fresh by an event
/// stream the way `battery`/`volume`/`hotspot` are.
fn hover_refresh_topics(widget: &str) -> &'static [&'static str] {
    match widget {
        // T29: one topic per stacked row, all refreshed together — the
        // popup is one box covering every row (`popup_multi`), so there is
        // no way to refresh only the row under the pointer.
        "sysload" => &["cpu-detail", "mem-detail"],
        "devload" => &["claude", "docker", "archupdate"],
        "clock" => &["clock-detail"],
        "date" => &["date-detail"],
        "wifi" => &["wifi-detail"],
        "eth" => &["eth-detail"],
        "netsec" => &["sec-detail"],
        "battery" => &["battery"],
        "volume" => &["volume"],
        "hotspot" => &["hotspot"], // T15
        "remote" => &["remote"],
        "keepass" => &["keepass"], // T28
        // T28: `music` is already kept live off its own `playerctl
        // --follow` stream (music.rs) — same "never goes stale between
        // hovers" reasoning this doc comment gives for a workspace tag
        // pill's tip, not the "no ironvar at all" reasoning it gives for
        // bluetooth.
        "music" => &[],
        // Widget/class name ("inhibit") and refresh topic ("keepawake")
        // deliberately differ here: the pill kept its old `inhibit`
        // name/class to reuse the existing style.css selectors and
        // ironvars, but the collector, unit, and script are all named
        // `keepawake` (keepawake.rs/keepawake.sh/mango-keepawake.service).
        // Mapping straight to `&[widget]` like the group above would
        // dispatch an unmatched "inhibit" topic into the catch-all resync
        // arm instead of the targeted one.
        "inhibit" => &["keepawake"],
        _ => &[],
    }
}

/// `Hover::Enter`'s pure state transition. Returns `Some(bar)` when a
/// *different* widget's popup was already open and must be hidden now (the
/// mouse jumped pill to pill with no gap) — the actual `ipc.hide_popup`
/// call stays in the select loop so this branching is unit-testable without
/// a live ironbar socket.
///
/// T-popup-hold: always cancels `hover_hide_due` first — any real hover
/// activity (re-hovering the open pill, or jumping to a different one)
/// supersedes a pending grace-hide from a just-abandoned `Hover::Exit`.
fn hover_enter(
    key: (String, String),
    hover_pending: &mut Option<(String, String)>,
    hover_due: &mut Option<Instant>,
    hover_open: &mut Option<(String, String)>,
    hover_hide_due: &mut Option<Instant>,
) -> Option<String> {
    *hover_hide_due = None;
    if hover_open.as_ref() == Some(&key) {
        // Already showing this exact (bar, widget): no-op, not a re-arm —
        // a re-hover of the same pill must not restart the delay.
        return None;
    }
    let hide = hover_open.take().map(|(old_bar, _)| old_bar);
    *hover_pending = Some(key);
    *hover_due = Some(Instant::now() + HOVER_DELAY);
    hide
}

/// `Hover::Exit`'s pure state transition.
///
/// T-popup-hold: no longer hides on the spot when the popup was open —
/// the pointer may be headed across `popup_gap` into the popup itself
/// (bluetooth's own "can never reach it" report), which `on_mouse_exit`
/// cannot tell apart from actually leaving for good. Arms
/// `hover_hide_due` instead; the select loop closes it if nothing cancels
/// that deadline first (`hover_enter` on any pill, or `hover_hold` on the
/// popup). `hover_open` itself is left untouched — the popup is still
/// genuinely showing until the grace period actually elapses.
fn hover_exit(
    key: (String, String),
    hover_pending: &mut Option<(String, String)>,
    hover_due: &mut Option<Instant>,
    hover_open: &Option<(String, String)>,
    hover_hide_due: &mut Option<Instant>,
) {
    if hover_pending.as_ref() == Some(&key) {
        // Still waiting, never opened — cancel outright. This is the whole
        // point of a cancelable debounce over a shell sleep.
        *hover_pending = None;
        *hover_due = None;
    }
    if hover_open.as_ref() == Some(&key) {
        *hover_hide_due = Some(Instant::now() + HOVER_HIDE_GRACE);
    }
}

/// `Hover::Hold`'s pure state transition — the pointer reached the popup's
/// own content box (genconfig.rs's `popup()` wires this to the outer
/// vertical box's `on_mouse_enter`). Cancels a pending grace-hide, the same
/// "still here" signal a pill re-hover already gives via `hover_enter`.
///
/// `bar` is checked against `hover_open` rather than trusted outright: the
/// popup box carries no widget name of its own (see `HoverEvent`'s doc
/// comment), so this is the only cross-check available against acting on a
/// stale event for a bar that isn't the one currently shown.
fn hover_hold(
    bar: &str,
    hover_open: &Option<(String, String)>,
    hover_hide_due: &mut Option<Instant>,
) {
    if hover_open.as_ref().is_some_and(|(b, _)| b == bar) {
        *hover_hide_due = None;
    }
}

/// `Hover::Release`'s pure state transition — the pointer left the popup's
/// own content box after previously entering it (`Hold`). Arms the same
/// grace-hide deadline `hover_exit` arms for a pill leave, so leaving the
/// popup behaves exactly like leaving the pill it came from: one more short
/// window to come back before the popup actually closes.
fn hover_release(
    bar: &str,
    hover_open: &Option<(String, String)>,
    hover_hide_due: &mut Option<Instant>,
) {
    if hover_open.as_ref().is_some_and(|(b, _)| b == bar) {
        *hover_hide_due = Some(Instant::now() + HOVER_HIDE_GRACE);
    }
}

async fn client_cmd(cmd: &str, quiet: bool) -> Result<(), String> {
    let resp = control::send(cmd).await.map_err(|e| e.to_string())?;
    if !quiet {
        println!("{resp}");
    }
    Ok(())
}

async fn run() -> Result<(), String> {
    sys::set_timer_slack(ECO_TIMER_SLACK_MS);

    let clock = Clock::new().map_err(|e| format!("clock timerfd: {e}"))?;
    let clock_fd = AsyncFd::new(clock).map_err(|e| format!("clock AsyncFd: {e}"))?;

    let pm = PowermodeWatch::new().map_err(|e| format!("powermode watch: {e}"))?;
    let mut pm_mode = pm.mode;
    let mut pm_fd = AsyncFd::new(pm).map_err(|e| format!("powermode AsyncFd: {e}"))?;

    // T28: same shape as `pm`/`pm_fd` above — one file's inode, no fork ever.
    let archupdate = Archupdate::new().map_err(|e| format!("arch-update watch: {e}"))?;
    let mut archupdate_fd =
        AsyncFd::new(archupdate).map_err(|e| format!("arch-update AsyncFd: {e}"))?;

    let control_listener = control::listen().map_err(|e| format!("control socket: {e}"))?;
    let control_path = control::socket_path();

    let mut wheel = Wheel::new().map_err(|e| format!("wheel timerfd: {e}"))?;
    wheel
        .set_mode(pm_mode)
        .map_err(|e| format!("wheel arm: {e}"))?;
    let mut wheel_fd = AsyncFd::new(wheel).map_err(|e| format!("wheel AsyncFd: {e}"))?;

    let pomo = Pomo::new().map_err(|e| format!("pomo timerfd: {e}"))?;
    let mut pomo_fd = AsyncFd::new(pomo).map_err(|e| format!("pomo AsyncFd: {e}"))?;
    pomo_fd.get_mut().catch_up_on_start().await;

    let mut mango = Mango::new();
    let mut net = Net::new();
    let mut audio = Audio::new();
    let mut power = Power::new();
    let mut cpu = Cpu::new();
    let mut mem = Memory::new();
    let mut docker = Docker::new();
    let mut hotspot = Hotspot::new();
    let mut remote = Remote::new();
    let mut keepawake = Keepawake::new();
    let mut darkmode = Darkmode::new();
    let mut claude = Claude::new();
    let mut keepass = Keepass::new();
    let mut music = Music::new();
    let mut vars = Vars::new();
    let mut ipc = IronbarIpc::new();
    let mut stats = Stats::new();
    let mut dirty_since: Option<Instant> = None;
    // Debounces regrade() itself (forks), separate from `dirty_since` (which
    // debounces the IPC flush of whatever regrade() already computed) — see
    // net.rs module doc and IRONBAR.md T3 "Architecture".
    let mut regrade_due: Option<Instant> = None;
    // Same shape as `regrade_due`, for audio.rs's `refresh()` (also forks).
    let mut audio_due: Option<Instant> = None;
    // Same shape again, for power.rs's `refresh()`.
    let mut power_due: Option<Instant> = None;
    // Same shape again, for docker.rs's `refresh()` — a `docker compose up`
    // can start several containers in a burst, each its own event line.
    // Same shape again, for keepass.rs's `refresh()`. Its trigger arm below
    // filters to real signals only (`keepass::is_signal_line`) so this
    // debounce collapses a genuine signal burst — it no longer has to stop
    // a feedback loop, that was T-keepass-loop and is fixed at the filter.
    let mut keepass_due: Option<Instant> = None;
    // hotspot.rs/darkmode.rs need no `_due` var: neither has an event stream
    // that can burst (T6b D2/D3) — their refreshes are called directly,
    // inline, from the control socket and (hotspot only) the clock tick.
    let mut docker_due: Option<Instant> = None;

    // Hover-to-open state (T-hover): one pending slot, matching the shape of
    // every `_due` var above — a mouse can only be hovering one bar pill at
    // a time, so one slot is correct, not a per-hover spawned task or a
    // shell `sleep` (neither of which `on_mouse_exit` could cancel).
    // `hover_due` already holds its own fire-at deadline (unlike the
    // `_due` vars above, which store the event time and add
    // `FLUSH_DEBOUNCE` at the sleep_until call site) — see the select arm
    // below.
    let mut hover_due: Option<Instant> = None;
    // What the pending open, once `hover_due` elapses, will show:
    // (bar, widget).
    let mut hover_pending: Option<(String, String)> = None;
    // What is actually shown right now — lets Hover::Exit tell "still
    // waiting, cancel" apart from "already open, hide": (bar, widget).
    let mut hover_open: Option<(String, String)> = None;
    // T-popup-hold: when set, the bar in `hover_open` closes once this
    // deadline elapses, unless `hover_enter`/`hover_hold` cancels it first
    // — see `hover_exit`'s own doc comment for why this replaced an
    // immediate hide.
    let mut hover_hide_due: Option<Instant> = None;

    clock::refresh(&mut vars);
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }

    pomo_fd.get_ref().refresh(&mut vars);
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }

    // Initial full picture at startup — net.rs's `MonitorChild` has no
    // `mmsg get`-style snapshot pairing, so this stands in for one.
    net.regrade(&mut vars, pm_mode).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }

    // Same reason: a `pactl subscribe` stream has no snapshot companion.
    audio.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }

    // Same reason again, plus priming power.rs's own AC-edge detector so the
    // first real refresh below doesn't read as a plug/unplug edge — see
    // Power::prime's doc comment.
    power.prime();
    power.refresh(&mut vars, pm_mode).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    power.trigger_startup_mode();

    // T6a: cpu needs a short second sample to avoid a blank first reading
    // (cpu.sh:344-350's own fix for the same problem); memory is
    // instantaneous, so a plain refresh is already correct.
    cpu.prime(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    mem.refresh(&mut vars);
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }

    // T6b: docker/hotspot/darkmode all need an explicit startup refresh —
    // none has a snapshot companion (docker's `MonitorChild` supplies no
    // "current state" query, same reasoning as net.rs's own priming note
    // above; hotspot/darkmode have no event stream at all).
    docker.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    hotspot.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    remote.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    keepawake.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    darkmode.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }

    // T6c: render whatever claudebar's cache already holds (likely stale or
    // absent on a cold start), then kick off a real fetch in the background
    // so the pill has fresh data soon rather than waiting for the next
    // 5-minute-aligned tick.
    claude.refresh(&mut vars);
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    claude::spawn_fetch();

    // T28: keepass/archupdate need an explicit startup refresh (no
    // snapshot companion — hotspot/darkmode's own reasoning above applies
    // here too); archupdate is a pure file read like darkmode's own.
    keepass.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }
    archupdate_fd.get_ref().refresh(&mut vars);
    if vars.has_dirty() {
        dirty_since.get_or_insert_with(Instant::now);
    }

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| format!("sigterm handler: {e}"))?;

    eprintln!("mango-bard: running, mode={}", pm_mode.as_str());

    loop {
        stats.wakeups += 1;

        // The eco slack (500ms) is applied by the kernel to this loop's own
        // epoll_wait timeout, so any armed `_due` deadline can fire that
        // late — T13 measured ~650-850ms on `hover_due` before scoping the
        // slack to idle. `dirty_since` was left out of that fix and still
        // paid the full slack, which is what made a workspace switch repaint
        // late. Tight whenever any deadline is armed: a pending flush or
        // hover means a real event is in flight, so there is no idle wakeup
        // left to coalesce with. Replaces the four separate restore sites
        // this subsumed (each of the arms below used to set this back after
        // its own deadline fired or was cancelled) — one call, correct by
        // construction, instead of four call sites that all had to remember.
        sys::set_timer_slack(
            if dirty_since.is_some() || hover_due.is_some() || hover_hide_due.is_some() {
                0
            } else {
                ECO_TIMER_SLACK_MS
            },
        );

        tokio::select! {
            r = clock_fd.readable() => {
                match r {
                    Ok(mut guard) => {
                        guard.get_inner().on_tick(&mut vars);
                        guard.clear_ready();
                        if vars.has_dirty() {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                        // T7c: the pomodoro's minute-granular countdown and
                        // its break-time nag ride this same tick — no timer
                        // of their own beyond the one phase-boundary timerfd
                        // (pomo_fd's own select arm, below).
                        pomo_fd.get_mut().on_minute(&mut vars).await;
                        if vars.has_dirty() {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                        // Decision 2 (IRONBAR.md T3): RSSI rides this tick,
                        // gated by powermode — no timer of its own.
                        if net.should_refresh_on_tick(pm_mode) {
                            net.regrade(&mut vars, pm_mode).await;
                            if vars.has_dirty() {
                                dirty_since.get_or_insert_with(Instant::now);
                            }
                        }
                        // power.rs's 5-minute discharging backstop — no
                        // pm_mode gate, it is a staleness bound, not a poll.
                        if power.should_refresh_on_tick() {
                            power_due = Some(Instant::now());
                        }
                        // hotspot.rs's client-count backstop (T6b D2) — gated
                        // on `up`, so this is a no-op almost always. Called
                        // directly (not via a `_due` debounce) since the
                        // clock's own 1/min cadence already bounds it.
                        if hotspot.should_refresh_on_tick() {
                            hotspot.refresh(&mut vars).await;
                            if vars.has_dirty() {
                                dirty_since.get_or_insert_with(Instant::now);
                            }
                        }
                        // T6c: claudebar's own 300s-aligned fetch, riding
                        // this tick instead of a dedicated timerfd — see
                        // claude.rs::due_on_tick. The fetch itself runs in
                        // the background (claude::spawn_fetch), so this
                        // never blocks the tick handler.
                        if claude::due_on_tick() {
                            claude::spawn_fetch();
                        }
                        // T6a eco path: the wheel is disarmed in eco (see
                        // wheel.rs module doc), so cpu/memory ride this tick
                        // instead — mirrors net.rs's RSSI gate above.
                        let due = wheel_fd.get_ref().on_minute(pm_mode);
                        if due.cpu {
                            cpu.refresh(&mut vars);
                            stats.cpu_polls += 1;
                        }
                        if due.mem {
                            mem.refresh(&mut vars);
                            stats.mem_polls += 1;
                        }
                        if (due.cpu || due.mem) && vars.has_dirty() {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                    }
                    Err(e) => eprintln!("mango-bard: clock fd error: {e}"),
                }
            }

            r = wheel_fd.readable_mut() => {
                match r {
                    Ok(mut guard) => {
                        stats.wheel_ticks += 1;
                        let due = guard.get_inner_mut().on_tick();
                        guard.clear_ready();
                        if due.cpu {
                            cpu.refresh(&mut vars);
                            stats.cpu_polls += 1;
                        }
                        if due.mem {
                            mem.refresh(&mut vars);
                            stats.mem_polls += 1;
                        }
                        if vars.has_dirty() {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                    }
                    Err(e) => eprintln!("mango-bard: wheel fd error: {e}"),
                }
            }

            r = pomo_fd.readable_mut() => {
                match r {
                    Ok(mut guard) => {
                        guard.get_inner_mut().on_boundary(&mut vars).await;
                        guard.clear_ready();
                        if vars.has_dirty() {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                    }
                    Err(e) => eprintln!("mango-bard: pomo fd error: {e}"),
                }
            }

            line = net.nmcli_mon.next_line() => {
                stats.net_events += 1;
                if net.ingest_nmcli_line(&line) {
                    net.refresh_busy_var(&mut vars);
                    if vars.has_dirty() {
                        dirty_since.get_or_insert_with(Instant::now);
                    }
                    regrade_due = Some(Instant::now());
                } else {
                    stats.net_noop += 1;
                }
            }

            line = net.route_mon.next_line() => {
                stats.net_events += 1;
                if net.ingest_route_line(&line) {
                    regrade_due = Some(Instant::now());
                } else {
                    stats.net_noop += 1;
                }
            }

            line = audio.pactl_mon.next_line() => {
                stats.audio_events += 1;
                if audio.ingest_line(&line) {
                    audio_due = Some(Instant::now());
                } else {
                    stats.audio_noop += 1;
                }
            }

            _line = power.udev_mon.next_line() => {
                // No dedup gate — see Stats::power_events doc comment: every
                // udevadm monitor line (including its own startup banner) is
                // regrade-worthy, matching ac-watch.sh's "event count, not
                // content" edge detector.
                stats.power_events += 1;
                power_due = Some(Instant::now());
            }

            line = docker.events_mon.next_line() => {
                stats.docker_events += 1;
                if docker.ingest_line(&line) {
                    docker_due = Some(Instant::now());
                } else {
                    stats.docker_noop += 1;
                }
            }

            // T28: `busctl --user monitor` line. Only a genuine `signal`
            // re-arms the debounce — `keepass::is_signal_line`'s doc
            // comment has the measured feedback-loop rate this filter
            // breaks (T-keepass-loop). `keepass_due`'s 150 ms debounce
            // still collapses a real signal burst to one refresh.
            line = keepass.mon.next_line() => {
                if keepass::is_signal_line(&line) {
                    keepass_due = Some(Instant::now());
                }
            }

            // T28: `playerctl --follow` line — already the fully-parsed
            // state (music.rs's own doc comment), so this is render only,
            // no fork, no debounce needed.
            line = music.mon.next_line() => {
                if music.ingest_line(&line) {
                    music.apply(&mut vars);
                    if vars.has_dirty() {
                        dirty_since.get_or_insert_with(Instant::now);
                    }
                }
            }

            line = mango.monitors.next_doc() => {
                stats.mmsg_events += 1;
                if mango.ingest_monitors(&line) {
                    mango.apply(&mut vars);
                    if vars.has_dirty() {
                        dirty_since.get_or_insert_with(Instant::now);
                    }
                } else {
                    stats.mmsg_noop += 1;
                }
            }

            line = mango.clients.next_doc() => {
                stats.mmsg_events += 1;
                if mango.ingest_clients(&line) {
                    mango.apply(&mut vars);
                    if vars.has_dirty() {
                        dirty_since.get_or_insert_with(Instant::now);
                    }
                } else {
                    stats.mmsg_noop += 1;
                }
            }

            r = pm_fd.readable_mut() => {
                match r {
                    Ok(mut guard) => {
                        let changed = guard.get_inner_mut().on_event();
                        guard.clear_ready();
                        if let Some(new_mode) = changed {
                            pm_mode = new_mode;
                            eprintln!("mango-bard: powermode -> {}", new_mode.as_str());
                            // Pure class recompute, no forks — the netsec
                            // verdict itself doesn't change on a mode flip.
                            net.set_eco_class(&mut vars, pm_mode);
                            if vars.has_dirty() {
                                dirty_since.get_or_insert_with(Instant::now);
                            }
                            // power.rs's eco leaf and Mode row are drawn
                            // from pm_mode — repaint on the flip. Safe from
                            // a feedback loop: refresh() only fires the AC
                            // edge action when `online` itself changed, and
                            // a mode flip alone never changes that.
                            power_due = Some(Instant::now());
                            // T6a: re-program the wheel for the new mode
                            // (period table in wheel.rs); disarms outright
                            // in eco, which is what keeps goal 2 intact.
                            match wheel_fd.get_mut().set_mode(pm_mode) {
                                Ok(true) => eprintln!("mango-bard: wheel re-programmed for {}", new_mode.as_str()),
                                Ok(false) => {}
                                Err(e) => eprintln!("mango-bard: wheel re-arm error: {e}"),
                            }
                        }
                    }
                    Err(e) => eprintln!("mango-bard: powermode fd error: {e}"),
                }
            }

            // T28: same shape as `pm_fd` above, minus the mode-change
            // branching — every event just means "re-read the two count
            // files", which `refresh()` (a pure file read, no fork) does
            // unconditionally.
            r = archupdate_fd.readable_mut() => {
                match r {
                    Ok(mut guard) => {
                        let inner = guard.get_inner_mut();
                        inner.on_event();
                        inner.refresh(&mut vars);
                        guard.clear_ready();
                        if vars.has_dirty() {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                    }
                    Err(e) => eprintln!("mango-bard: arch-update fd error: {e}"),
                }
            }

            accepted = control_listener.accept() => {
                if let Ok((mut stream, _)) = accepted {
                    if let Ok(line) = control::read_line(&mut stream).await {
                        match control::parse(&line) {
                            Line::Pomo(verb, arg) => {
                                let reply = pomo_fd.get_mut().control(&verb, arg.as_deref(), &mut vars).await;
                                if vars.has_dirty() {
                                    dirty_since.get_or_insert_with(Instant::now);
                                }
                                let _ = control::reply(&mut stream, &reply).await;
                            }
                            Line::Ping => { let _ = control::reply(&mut stream, "ok").await; }
                            Line::Stats => { let _ = control::reply(&mut stream, &stats.to_json(pm_mode)).await; }
                            Line::Refresh(topic) => {
                                eprintln!("mango-bard: refresh requested: {topic}");
                                dispatch_refresh(
                                    &topic, &mut vars, &mut mango, &mut net, &mut audio,
                                    &mut power, &mut cpu, &mut mem, &mut docker, &mut hotspot,
                                    &mut remote, &mut keepawake, &mut darkmode, &mut claude,
                                    &mut keepass, archupdate_fd.get_ref(), pm_mode, &mut stats,
                                    &mut regrade_due, &mut audio_due, &mut power_due,
                                    &mut docker_due,
                                ).await;
                                if vars.has_dirty() {
                                    dirty_since.get_or_insert_with(Instant::now);
                                }
                                let _ = control::reply(&mut stream, "ok").await;
                            }
                            Line::Hover(event, bar, widget) => {
                                // hover_enter/hover_exit do the actual state
                                // mutation and return which bar (if any)
                                // needs an unconditional hide_popup — the IO
                                // stays here so the branching logic itself
                                // is unit-testable (see main.rs's own tests).
                                let hide = match event {
                                    HoverEvent::Enter => hover_enter(
                                        (bar, widget),
                                        &mut hover_pending,
                                        &mut hover_due,
                                        &mut hover_open,
                                        &mut hover_hide_due,
                                    ),
                                    HoverEvent::Exit => {
                                        hover_exit(
                                            (bar, widget),
                                            &mut hover_pending,
                                            &mut hover_due,
                                            &hover_open,
                                            &mut hover_hide_due,
                                        );
                                        None
                                    }
                                };
                                if let Some(hide_bar) = hide {
                                    let _ = ipc.hide_popup(&hide_bar).await;
                                }
                                // Timer slack for hover_due/hover_hide_due is
                                // now recomputed once at the top of the loop
                                // (see that comment) rather than restored
                                // here — the next iteration picks up whatever
                                // hover_enter/hover_exit just set.
                                let _ = control::reply(&mut stream, "ok").await;
                            }
                            Line::HoverPopup(event, bar) => {
                                // Hold/release never trigger an immediate
                                // hide_popup — they only arm/cancel
                                // `hover_hide_due` (T-popup-hold; see
                                // PopupHoverEvent's own doc comment).
                                match event {
                                    PopupHoverEvent::Hold => {
                                        hover_hold(&bar, &hover_open, &mut hover_hide_due)
                                    }
                                    PopupHoverEvent::Release => {
                                        hover_release(&bar, &hover_open, &mut hover_hide_due)
                                    }
                                }
                                let _ = control::reply(&mut stream, "ok").await;
                            }
                            Line::Unknown => { let _ = control::reply(&mut stream, "unknown").await; }
                        }
                    }
                }
            }

            _ = async {
                match regrade_due {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t + FLUSH_DEBOUNCE)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                net.regrade(&mut vars, pm_mode).await;
                if vars.has_dirty() {
                    dirty_since.get_or_insert_with(Instant::now);
                }
                regrade_due = None;
            }

            _ = async {
                match audio_due {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t + FLUSH_DEBOUNCE)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                audio.refresh(&mut vars).await;
                if vars.has_dirty() {
                    dirty_since.get_or_insert_with(Instant::now);
                }
                audio_due = None;
            }

            _ = async {
                match power_due {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t + FLUSH_DEBOUNCE)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                power.refresh(&mut vars, pm_mode).await;
                if vars.has_dirty() {
                    dirty_since.get_or_insert_with(Instant::now);
                }
                power_due = None;
            }

            _ = async {
                match docker_due {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t + FLUSH_DEBOUNCE)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                docker.refresh(&mut vars).await;
                if vars.has_dirty() {
                    dirty_since.get_or_insert_with(Instant::now);
                }
                docker_due = None;
            }

            _ = async {
                match keepass_due {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t + FLUSH_DEBOUNCE)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                keepass.refresh(&mut vars).await;
                if vars.has_dirty() {
                    dirty_since.get_or_insert_with(Instant::now);
                }
                keepass_due = None;
            }

            // Hover-open debounce (T-hover) — same single-slot shape as the
            // `_due` arms above, but `hover_due` already holds the fire-at
            // deadline (set at Hover::Enter), not an event time needing
            // `HOVER_DELAY` added here.
            _ = async {
                match hover_due {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                if let Some((bar, widget)) = hover_pending.take() {
                    let topics = hover_refresh_topics(&widget);
                    if !topics.is_empty() {
                        // T29: `sysload`/`devload` each carry more than one
                        // topic (one popup, several rows) — run them in
                        // sequence, not concurrently: each call only holds
                        // its `&mut` borrows for the statement it's in.
                        for topic in topics {
                            dispatch_refresh(
                                topic, &mut vars, &mut mango, &mut net, &mut audio,
                                &mut power, &mut cpu, &mut mem, &mut docker, &mut hotspot,
                                &mut remote, &mut keepawake, &mut darkmode, &mut claude,
                                &mut keepass, archupdate_fd.get_ref(), pm_mode, &mut stats,
                                &mut regrade_due, &mut audio_due, &mut power_due,
                                &mut docker_due,
                            ).await;
                        }
                    } else if widget == "pomo" {
                        // Passive peek only — `pomo.refresh()` is a pure
                        // render from already-current state, never the
                        // mutating `pomo click`/`toggle` verb. Hover must
                        // never trigger a mutating action.
                        pomo_fd.get_ref().refresh(&mut vars);
                    }
                    // T13: flush synchronously here, before show_popup,
                    // instead of only arming `dirty_since` and leaving the
                    // fresh tip content to land on the next `FLUSH_DEBOUNCE`
                    // tick — that gap used to show the *previous* tip for
                    // 150ms after the popup was already visible.
                    if vars.has_dirty() {
                        flush_vars(&mut vars, &mut ipc, &mut stats).await;
                        dirty_since = if vars.has_dirty() { Some(Instant::now()) } else { None };
                    }
                    match ipc.show_popup(&bar, &widget).await {
                        Ok(()) => hover_open = Some((bar, widget)),
                        Err(e) => {
                            stats.ipc_errors += 1;
                            eprintln!("mango-bard: hover show_popup error: {e}");
                        }
                    }
                }
                hover_due = None;
                // Timer slack is recomputed once at the top of the loop now
                // (see that comment) — no restore needed here.
            }

            // Popup grace-hide (T-popup-hold) — same single-slot shape as
            // `hover_due` above. Fires `HOVER_HIDE_GRACE` after a pill or
            // the popup itself was left with nothing cancelling it first
            // (see `hover_exit`/`hover_release`'s own doc comments).
            _ = async {
                match hover_hide_due {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                if let Some((bar, _)) = hover_open.take() {
                    let _ = ipc.hide_popup(&bar).await;
                }
                hover_hide_due = None;
                // Timer slack is recomputed once at the top of the loop now
                // (see that comment) — no restore needed here.
            }

            _ = async {
                match dirty_since {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t + FLUSH_DEBOUNCE)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                flush_vars(&mut vars, &mut ipc, &mut stats).await;
                dirty_since = if vars.has_dirty() { Some(Instant::now()) } else { None };
            }

            _ = sigterm.recv() => {
                eprintln!("mango-bard: SIGTERM, shutting down");
                // FOCUS.md §5.3: a dead daemon must not leave every
                // notification silenced indefinitely — release unconditionally,
                // a no-op (makoctl mode -r on a mode that isn't active) if
                // nothing had DND on.
                pomo_fd.get_ref().release_dnd().await;
                // A silent `mmsg watch` never takes SIGPIPE and leaks
                // forever (mango.rs: Watch::kill doc comment) — kill_on_drop
                // alone isn't enough since nothing drops `mango` before exit.
                mango.monitors.kill();
                mango.clients.kill();
                net.nmcli_mon.kill();
                net.route_mon.kill();
                audio.pactl_mon.kill();
                power.udev_mon.kill();
                docker.events_mon.kill();
                keepass.mon.kill();
                music.mon.kill();
                control::unlink(&control_path);
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(bar: &str, widget: &str) -> (String, String) {
        (bar.to_string(), widget.to_string())
    }

    #[test]
    fn enter_then_exit_before_the_delay_elapses_cancels_the_pending_open() {
        let mut pending = None;
        let mut due = None;
        let mut open = None;
        let mut hide_due = None;
        let hide = hover_enter(
            key("bar-eDP-1", "cpu"),
            &mut pending,
            &mut due,
            &mut open,
            &mut hide_due,
        );
        assert_eq!(hide, None);
        assert_eq!(pending, Some(key("bar-eDP-1", "cpu")));
        assert!(due.is_some());

        hover_exit(
            key("bar-eDP-1", "cpu"),
            &mut pending,
            &mut due,
            &open,
            &mut hide_due,
        );
        assert_eq!(pending, None);
        assert_eq!(due, None);
        assert_eq!(open, None);
        assert_eq!(
            hide_due, None,
            "never opened, so there is nothing to grace-hide"
        );
    }

    #[test]
    fn enter_while_a_different_widget_is_open_hides_the_old_one_and_pends_the_new() {
        let mut pending = None;
        let mut due = None;
        let mut open = Some(key("bar-eDP-1", "cpu"));
        // T-popup-hold: a stale grace-hide (e.g. left over from the widget
        // this Enter is about to replace) must be cancelled unconditionally.
        let mut hide_due = Some(Instant::now());

        let hide = hover_enter(
            key("bar-eDP-1", "battery"),
            &mut pending,
            &mut due,
            &mut open,
            &mut hide_due,
        );
        assert_eq!(
            hide,
            Some("bar-eDP-1".to_string()),
            "the previously open widget's bar must be hidden"
        );
        assert_eq!(open, None);
        assert_eq!(pending, Some(key("bar-eDP-1", "battery")));
        assert!(due.is_some());
        assert_eq!(hide_due, None);
    }

    #[test]
    fn exit_on_an_already_open_popup_arms_the_grace_hide() {
        // T-popup-hold: no immediate hide — the pointer may be headed into
        // the popup itself (see hover_exit's own doc comment). hover_open
        // stays put; only a grace deadline gets armed.
        let mut pending = None;
        let mut due = None;
        let open = Some(key("bar-eDP-1", "docker"));
        let mut hide_due = None;

        hover_exit(
            key("bar-eDP-1", "docker"),
            &mut pending,
            &mut due,
            &open,
            &mut hide_due,
        );
        assert_eq!(open, Some(key("bar-eDP-1", "docker")), "still showing");
        assert!(hide_due.is_some(), "grace-hide must be armed");
    }

    #[test]
    fn re_entering_the_already_open_widget_is_a_no_op() {
        // A re-hover of the same pill must not restart the delay or touch
        // hover_open — but it must still cancel any grace-hide the matching
        // Exit had already armed (the whole point of the grace window).
        let mut pending = None;
        let mut due = None;
        let mut open = Some(key("bar-eDP-1", "volume"));
        let mut hide_due = Some(Instant::now());

        let hide = hover_enter(
            key("bar-eDP-1", "volume"),
            &mut pending,
            &mut due,
            &mut open,
            &mut hide_due,
        );
        assert_eq!(hide, None);
        assert_eq!(pending, None, "no pending open should be armed");
        assert_eq!(due, None);
        assert_eq!(open, Some(key("bar-eDP-1", "volume")));
        assert_eq!(hide_due, None);
    }

    #[test]
    fn exit_on_an_unrelated_widget_touches_nothing() {
        let mut pending = Some(key("bar-eDP-1", "cpu"));
        let mut due = Some(Instant::now());
        let open = None;
        let mut hide_due = None;

        hover_exit(
            key("bar-eDP-1", "bluetooth"),
            &mut pending,
            &mut due,
            &open,
            &mut hide_due,
        );
        assert_eq!(pending, Some(key("bar-eDP-1", "cpu")));
        assert!(due.is_some());
        assert_eq!(hide_due, None);
    }

    #[test]
    fn hold_cancels_the_grace_hide_for_the_open_bar() {
        let open = Some(key("bar-eDP-1", "cpu"));
        let mut hide_due = Some(Instant::now());
        hover_hold("bar-eDP-1", &open, &mut hide_due);
        assert_eq!(hide_due, None);
    }

    #[test]
    fn hold_on_a_different_bar_than_the_open_one_touches_nothing() {
        // Defensive cross-check (HoverPopup carries no widget name) — a
        // stale/mismatched bar must not cancel another bar's grace-hide.
        let open = Some(key("bar-eDP-1", "cpu"));
        let mut hide_due = Some(Instant::now());
        hover_hold("bar-default", &open, &mut hide_due);
        assert!(hide_due.is_some());
    }

    #[test]
    fn release_arms_the_grace_hide_for_the_open_bar() {
        let open = Some(key("bar-eDP-1", "cpu"));
        let mut hide_due = None;
        hover_release("bar-eDP-1", &open, &mut hide_due);
        assert!(hide_due.is_some());
    }

    #[test]
    fn release_on_a_different_bar_than_the_open_one_touches_nothing() {
        let open = Some(key("bar-eDP-1", "cpu"));
        let mut hide_due = None;
        hover_release("bar-default", &open, &mut hide_due);
        assert_eq!(hide_due, None);
    }

    #[test]
    fn hover_refresh_topics_maps_lazy_detail_and_alias_widgets() {
        // T29: one merged popup per stack, one topic per row.
        assert_eq!(
            hover_refresh_topics("sysload"),
            ["cpu-detail", "mem-detail"]
        );
        assert_eq!(
            hover_refresh_topics("devload"),
            ["claude", "docker", "archupdate"]
        );
        assert_eq!(hover_refresh_topics("clock"), ["clock-detail"]);
        assert_eq!(hover_refresh_topics("date"), ["date-detail"]);
        assert_eq!(hover_refresh_topics("battery"), ["battery"]);
        assert_eq!(hover_refresh_topics("volume"), ["volume"]);
        assert_eq!(hover_refresh_topics("hotspot"), ["hotspot"]); // T15
        assert_eq!(hover_refresh_topics("remote"), ["remote"]);
        // T19
        assert_eq!(hover_refresh_topics("wifi"), ["wifi-detail"]);
        assert_eq!(hover_refresh_topics("eth"), ["eth-detail"]);
        assert_eq!(hover_refresh_topics("netsec"), ["sec-detail"]);
        // T28
        assert_eq!(hover_refresh_topics("keepass"), ["keepass"]);
    }

    #[test]
    fn hover_refresh_topics_skips_bluetooth_and_workspace_pills() {
        // bluetooth: native module, no daemon-tracked ironvar to refresh.
        // workspace pills: already kept fresh by mango.apply()'s own event
        // stream. Neither should fall through to the generic full resync.
        assert!(hover_refresh_topics("bluetooth").is_empty());
        assert!(hover_refresh_topics("ws-eDP-1-1").is_empty());
        assert!(hover_refresh_topics("pomo").is_empty()); // handled separately
                                                          // T28: music is already kept live off its own event stream.
        assert!(hover_refresh_topics("music").is_empty());
    }
}
