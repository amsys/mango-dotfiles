//! mango-bard — event-driven daemon feeding ironbar's ironvars. T1 scope:
//! clock + powermode watcher + control socket + ironbar IPC client. See
//! /home/martin/work/mango-dotfiles/IRONBAR.md for the full design and the
//! T0 spike findings this implementation is built on.

mod clock;
mod control;
mod genconfig;
mod ipc;
mod mango;
mod net;
mod powermode;
mod routes;
mod sys;
mod tooltip;
mod vars;

use clock::Clock;
use control::Line;
use ipc::IronbarIpc;
use mango::Mango;
use net::Net;
use powermode::PowermodeWatch;
use std::time::{Duration, Instant};
use tokio::io::unix::AsyncFd;
use vars::Vars;

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
            started: Instant::now(),
        }
    }

    fn to_json(&self, mode: powermode::Mode) -> String {
        format!(
            "{{\"wakeups\":{},\"var_sets\":{},\"flushes\":{},\"ipc_errors\":{},\"mmsg_events\":{},\"mmsg_noop\":{},\"net_events\":{},\"net_noop\":{},\"style_sets\":{},\"uptime_s\":{},\"mode\":\"{}\"}}",
            self.wakeups,
            self.var_sets,
            self.flushes,
            self.ipc_errors,
            self.mmsg_events,
            self.mmsg_noop,
            self.net_events,
            self.net_noop,
            self.style_sets,
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

    let mut mango = Mango::new();
    let mut net = Net::new();
    let mut vars = Vars::new();
    let mut ipc = IronbarIpc::new();
    let mut stats = Stats::new();
    let mut dirty_since: Option<Instant> = None;
    // Debounces regrade() itself (forks), separate from `dirty_since` (which
    // debounces the IPC flush of whatever regrade() already computed) — see
    // net.rs module doc and IRONBAR.md T3 "Architecture".
    let mut regrade_due: Option<Instant> = None;

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
                    }
                    Err(e) => eprintln!("mango-bard: clock fd error: {e}"),
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
                                    _ => {
                                        clock::refresh(&mut vars);
                                        mango.apply(&mut vars);
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
                control::unlink(&control_path);
                return Ok(());
            }
        }
    }
}
