//! The daemon's own control channel: `$XDG_RUNTIME_DIR/mango-bard.sock`, one
//! ASCII line per connection. Chosen over `SIGUSR1` + a topic file (the
//! option IRONBAR.md left open) because a signal carries no payload — the
//! topic would still need a side file, adding a filesystem write per poke
//! and a lost-update race if two callers poke in the same instant. This is
//! ~20 lines with tokio::net::UnixListener (already in the dependency set)
//! and is race-free.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub fn socket_path() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("mango-bard.sock")
}

pub fn listen() -> io::Result<UnixListener> {
    let path = socket_path();
    // Restart=on-failure can leave a stale node behind; bind() would
    // otherwise fail with AddrInUse against a socket nothing is listening on.
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(&path)
}

pub enum Line {
    Refresh(String),
    Ping,
    Stats,
    Unknown,
}

pub fn parse(line: &str) -> Line {
    let line = line.trim();
    if let Some(topic) = line.strip_prefix("refresh ") {
        Line::Refresh(topic.trim().to_string())
    } else if line == "ping" {
        Line::Ping
    } else if line == "stats" {
        Line::Stats
    } else {
        Line::Unknown
    }
}

pub async fn read_line(stream: &mut UnixStream) -> io::Result<String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    Ok(line)
}

pub async fn reply(stream: &mut UnixStream, text: &str) -> io::Result<()> {
    stream.write_all(text.as_bytes()).await?;
    stream.write_all(b"\n").await
}

/// Client half — used by `mango-bard refresh <topic>` / `stats` / `ping`.
pub async fn send(cmd: &str) -> io::Result<String> {
    let path = socket_path();
    let mut stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(&path))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connecting to mango-bard"))??;
    stream.write_all(cmd.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.shutdown().await.ok();
    let mut resp = String::new();
    use tokio::io::AsyncReadExt;
    stream.read_to_string(&mut resp).await?;
    Ok(resp.trim().to_string())
}

pub fn unlink(path: &Path) {
    let _ = std::fs::remove_file(path);
}
