//! Shared, timeout-wrapping external-command helpers. bard is a
//! single-threaded `tokio::select!` loop (main.rs) — every collector that
//! forks `nmcli`/`iw`/`docker`/`pactl`/`systemctl`/... and awaits the child
//! inline shares that one loop, so one stalled fork (a wedged docker daemon,
//! a NetworkManager stuck mid Wi-Fi handoff) used to freeze the whole bar —
//! clock, popups, every pill — indefinitely. `ipc.rs`'s `ROUND_TRIP_TIMEOUT`
//! already carries a hard deadline for exactly this reason; this module gives
//! every collector fork the same one.
//!
//! `run()` and `is_active()` were duplicated byte-identically across
//! net.rs/audio.rs and remote.rs/keepawake.rs before this module existed.
//! Every caller already treats a failed fork as "no data" (`String::new()`)
//! or "not active" (`false`), so timing out degrades to the same value a
//! spawn failure already produced — no new error plumbing needed.

use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

/// Exported so a caller that needs the raw `Output`/`ExitStatus` (docker.rs's
/// `refresh`, which distinguishes "daemon unreachable" from "empty fleet" by
/// exit status) can wrap its own `Command` in the same budget rather than
/// inventing a different one.
pub const CMD_TIMEOUT: Duration = Duration::from_secs(2);

/// Runs `cmd args...`, returning stdout as lossy UTF-8, or an empty string on
/// spawn failure, non-zero-but-unchecked exit (callers already tolerate
/// partial/empty output), or a timeout.
pub async fn run(cmd: &str, args: &[&str]) -> String {
    match timeout(CMD_TIMEOUT, Command::new(cmd).args(args).output()).await {
        Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(Err(_)) | Err(_) => String::new(),
    }
}

/// Same as [`run`], with `env` set on the child only (`Command::env` —
/// scoped to that one process, never the daemon's own environment). Only
/// caller today is clock.rs's `zone_snapshot`, which forks `date` with a
/// per-zone `TZ` rather than mutating the daemon's process-wide one.
pub async fn run_with_env(cmd: &str, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut c = Command::new(cmd);
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    match timeout(CMD_TIMEOUT, c.output()).await {
        Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(Err(_)) | Err(_) => String::new(),
    }
}

/// `systemctl --user is-active <unit>` as a plain bool — matches the exit
/// code, not the printed status text, so a masked/failed unit reads as down
/// the same as a stopped one. A timeout also reads as down.
pub async fn is_active(unit: &str) -> bool {
    let fut = Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", unit])
        .status();
    matches!(timeout(CMD_TIMEOUT, fut).await, Ok(Ok(s)) if s.success())
}
