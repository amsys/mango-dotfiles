//! mango-bard — event-driven daemon feeding ironbar's ironvars. T1 scope:
//! clock + powermode watcher + control socket + ironbar IPC client. See
//! /home/martin/work/mango-dotfiles/IRONBAR.md for the full design and the
//! T0 spike findings this implementation is built on.

mod audio;
mod clock;
mod control;
mod cpu;
mod genconfig;
mod ipc;
mod mango;
mod memory;
mod net;
mod power;
mod powermode;
mod routes;
mod sys;
mod tooltip;
mod vars;
mod wheel;

use audio::Audio;
use clock::Clock;
use control::Line;
use cpu::Cpu;
use ipc::IronbarIpc;
use mango::Mango;
use memory::Memory;
use net::Net;
use power::Power;
use powermode::PowermodeWatch;
use std::time::{Duration, Instant};
use tokio::io::unix::AsyncFd;
use vars::Vars;
use wheel::Wheel;

const FLUSH_DEBOUNCE: Duration = Duration::from_millis(150);

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
            started: Instant::now(),
        }
    }

    fn to_json(&self, mode: powermode::Mode) -> String {
        format!(
            "{{\"wakeups\":{},\"var_sets\":{},\"flushes\":{},\"ipc_errors\":{},\"mmsg_events\":{},\"mmsg_noop\":{},\"net_events\":{},\"net_noop\":{},\"style_sets\":{},\"audio_events\":{},\"audio_noop\":{},\"power_events\":{},\"wheel_ticks\":{},\"cpu_polls\":{},\"mem_polls\":{},\"uptime_s\":{},\"mode\":\"{}\"}}",
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
            Some(topic) => client_cmd(&format!("refresh {topic}")).await,
            None => Err("usage: mango-bard refresh <topic>".into()),
        },
        Some("ping") => client_cmd("ping").await,
        Some("stats") => client_cmd("stats").await,
        Some("gen-config") => genconfig::main(&args[2..]).await,
        _ => Err("usage: mango-bard [run|refresh <topic>|ping|stats|gen-config]".into()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn client_cmd(cmd: &str) -> Result<(), String> {
    let resp = control::send(cmd).await.map_err(|e| e.to_string())?;
    println!("{resp}");
    Ok(())
}

async fn run() -> Result<(), String> {
    sys::set_timer_slack(500);

    let clock = Clock::new().map_err(|e| format!("clock timerfd: {e}"))?;
    let clock_fd = AsyncFd::new(clock).map_err(|e| format!("clock AsyncFd: {e}"))?;

    let pm = PowermodeWatch::new().map_err(|e| format!("powermode watch: {e}"))?;
    let mut pm_mode = pm.mode;
    let mut pm_fd = AsyncFd::new(pm).map_err(|e| format!("powermode AsyncFd: {e}"))?;

    let control_listener = control::listen().map_err(|e| format!("control socket: {e}"))?;
    let control_path = control::socket_path();

    let mut wheel = Wheel::new().map_err(|e| format!("wheel timerfd: {e}"))?;
    wheel
        .set_mode(pm_mode)
        .map_err(|e| format!("wheel arm: {e}"))?;
    let mut wheel_fd = AsyncFd::new(wheel).map_err(|e| format!("wheel AsyncFd: {e}"))?;

    let mut mango = Mango::new();
    let mut net = Net::new();
    let mut audio = Audio::new();
    let mut power = Power::new();
    let mut cpu = Cpu::new();
    let mut mem = Memory::new();
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

    clock::refresh(&mut vars);
    if vars.has_dirty() {
        dirty_since = Some(Instant::now());
    }

    // Initial full picture at startup — net.rs's `MonitorChild` has no
    // `mmsg get`-style snapshot pairing, so this stands in for one.
    net.regrade(&mut vars, pm_mode).await;
    if vars.has_dirty() {
        dirty_since = Some(Instant::now());
    }

    // Same reason: a `pactl subscribe` stream has no snapshot companion.
    audio.refresh(&mut vars).await;
    if vars.has_dirty() {
        dirty_since = Some(Instant::now());
    }

    // Same reason again, plus priming power.rs's own AC-edge detector so the
    // first real refresh below doesn't read as a plug/unplug edge — see
    // Power::prime's doc comment.
    power.prime();
    power.refresh(&mut vars, pm_mode).await;
    if vars.has_dirty() {
        dirty_since = Some(Instant::now());
    }
    power.trigger_startup_mode();

    // T6a: cpu needs a short second sample to avoid a blank first reading
    // (cpu.sh:344-350's own fix for the same problem); memory is
    // instantaneous, so a plain refresh is already correct.
    cpu.prime(&mut vars).await;
    if vars.has_dirty() {
        dirty_since = Some(Instant::now());
    }
    mem.refresh(&mut vars);
    if vars.has_dirty() {
        dirty_since = Some(Instant::now());
    }

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| format!("sigterm handler: {e}"))?;

    eprintln!("mango-bard: running, mode={}", pm_mode.as_str());

    loop {
        stats.wakeups += 1;

        tokio::select! {
            r = clock_fd.readable() => {
                match r {
                    Ok(mut guard) => {
                        guard.get_inner().on_tick(&mut vars);
                        guard.clear_ready();
                        if vars.has_dirty() {
                            dirty_since = Some(Instant::now());
                        }
                        // Decision 2 (IRONBAR.md T3): RSSI rides this tick,
                        // gated by powermode — no timer of its own.
                        if net.should_refresh_on_tick(pm_mode) {
                            net.regrade(&mut vars, pm_mode).await;
                            if vars.has_dirty() {
                                dirty_since = Some(Instant::now());
                            }
                        }
                        // power.rs's 5-minute discharging backstop — no
                        // pm_mode gate, it is a staleness bound, not a poll.
                        if power.should_refresh_on_tick() {
                            power_due = Some(Instant::now());
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
                            dirty_since = Some(Instant::now());
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
                            dirty_since = Some(Instant::now());
                        }
                    }
                    Err(e) => eprintln!("mango-bard: wheel fd error: {e}"),
                }
            }

            line = net.nmcli_mon.next_line() => {
                stats.net_events += 1;
                if net.ingest_nmcli_line(&line) {
                    net.refresh_busy_var(&mut vars);
                    if vars.has_dirty() {
                        dirty_since = Some(Instant::now());
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

            line = mango.monitors.next_doc() => {
                stats.mmsg_events += 1;
                if mango.ingest_monitors(&line) {
                    mango.apply(&mut vars);
                    if vars.has_dirty() {
                        dirty_since = Some(Instant::now());
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
                        dirty_since = Some(Instant::now());
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
                                dirty_since = Some(Instant::now());
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

            accepted = control_listener.accept() => {
                if let Ok((mut stream, _)) = accepted {
                    if let Ok(line) = control::read_line(&mut stream).await {
                        match control::parse(&line) {
                            Line::Ping => { let _ = control::reply(&mut stream, "ok").await; }
                            Line::Stats => { let _ = control::reply(&mut stream, &stats.to_json(pm_mode)).await; }
                            Line::Refresh(topic) => {
                                eprintln!("mango-bard: refresh requested: {topic}");
                                match topic.as_str() {
                                    "clock" => clock::refresh(&mut vars),
                                    "mango" | "workspaces" | "window" => mango.apply(&mut vars),
                                    "net" | "netsec" | "wifi" | "eth" | "wifi-scan" => {
                                        net.refresh_busy_var(&mut vars);
                                        regrade_due = Some(Instant::now());
                                    }
                                    "audio" | "volume" | "mic" => {
                                        audio_due = Some(Instant::now());
                                    }
                                    "power" | "battery" | "bat" | "ac" => {
                                        power_due = Some(Instant::now());
                                    }
                                    "cpu" => cpu.refresh(&mut vars),
                                    "memory" | "mem" => mem.refresh(&mut vars),
                                    _ => {
                                        clock::refresh(&mut vars);
                                        mango.apply(&mut vars);
                                        audio.refresh(&mut vars).await;
                                        power.refresh(&mut vars, pm_mode).await;
                                        cpu.refresh(&mut vars);
                                        mem.refresh(&mut vars);
                                    }
                                }
                                if vars.has_dirty() {
                                    dirty_since = Some(Instant::now());
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
                    dirty_since = Some(Instant::now());
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
                    dirty_since = Some(Instant::now());
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
                    dirty_since = Some(Instant::now());
                }
                power_due = None;
            }

            _ = async {
                match dirty_since {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t + FLUSH_DEBOUNCE)).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                stats.flushes += 1;
                if ipc.restarted() {
                    vars.mark_all_dirty();
                }
                loop {
                    let next = vars.peek_dirty().map(|(k, v)| (k.to_string(), v.to_string()));
                    let Some((k, v)) = next else { break };
                    // `@class/` keys never reach the wire as ironvars — they
                    // route to `style add-class`/`remove-class` instead
                    // (mango.rs: CLASS_PREFIX doc comment). A module may hold
                    // more than one independently dirty-tracked class (netsec
                    // needs both its verdict class and an independent `eco`
                    // class — IRONBAR.md T3): the key format is
                    // `@class/<module>[#<slot>]`, and only the module half
                    // (before `#`) is a real ironbar module name — the slot
                    // exists purely to keep the two keys apart in `Vars`.
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
                        Ok(()) => { vars.ack(&k); stats.var_sets += 1; }
                        Err(e) => { stats.ipc_errors += 1; eprintln!("mango-bard: ipc error setting {k}: {e}"); break; }
                    }
                }
                dirty_since = if vars.has_dirty() { Some(Instant::now()) } else { None };
            }

            _ = sigterm.recv() => {
                eprintln!("mango-bard: SIGTERM, shutting down");
                // A silent `mmsg watch` never takes SIGPIPE and leaks
                // forever (mango.rs: Watch::kill doc comment) — kill_on_drop
                // alone isn't enough since nothing drops `mango` before exit.
                mango.monitors.kill();
                mango.clients.kill();
                net.nmcli_mon.kill();
                net.route_mon.kill();
                audio.pactl_mon.kill();
                power.udev_mon.kill();
                control::unlink(&control_path);
                return Ok(());
            }
        }
    }
}
