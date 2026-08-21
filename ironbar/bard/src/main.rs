//! mango-bard — event-driven daemon feeding ironbar's ironvars. T1 scope:
//! clock + powermode watcher + control socket + ironbar IPC client. See
//! /home/martin/work/mango-dotfiles/IRONBAR.md for the full design and the
//! T0 spike findings this implementation is built on.

mod clock;
mod control;
mod ipc;
mod powermode;
mod sys;
mod vars;

use clock::Clock;
use control::Line;
use ipc::IronbarIpc;
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
    started: Instant,
}

impl Stats {
    fn new() -> Self {
        Self { wakeups: 0, var_sets: 0, flushes: 0, ipc_errors: 0, started: Instant::now() }
    }

    fn to_json(&self, mode: powermode::Mode) -> String {
        format!(
            "{{\"wakeups\":{},\"var_sets\":{},\"flushes\":{},\"ipc_errors\":{},\"uptime_s\":{},\"mode\":\"{}\"}}",
            self.wakeups,
            self.var_sets,
            self.flushes,
            self.ipc_errors,
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
        _ => Err("usage: mango-bard [run|refresh <topic>|ping|stats]".into()),
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

    let mut vars = Vars::new();
    let mut ipc = IronbarIpc::new();
    let mut stats = Stats::new();
    let mut dirty_since: Option<Instant> = None;

    clock::refresh(&mut vars);
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
                    }
                    Err(e) => eprintln!("mango-bard: clock fd error: {e}"),
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
                                clock::refresh(&mut vars);
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
                    match ipc.var_set(&k, &v).await {
                        Ok(()) => { vars.ack(&k); stats.var_sets += 1; }
                        Err(e) => { stats.ipc_errors += 1; eprintln!("mango-bard: ipc error setting {k}: {e}"); break; }
                    }
                }
                dirty_since = if vars.has_dirty() { Some(Instant::now()) } else { None };
            }

            _ = sigterm.recv() => {
                eprintln!("mango-bard: SIGTERM, shutting down");
                control::unlink(&control_path);
                return Ok(());
            }
        }
    }
}
