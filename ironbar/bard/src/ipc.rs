//! ironbar IPC client. Confirmed by T0 spike S4 (IRONBAR.md Spike findings):
//! one command per connection, newline-terminated JSON both ways, ~60us
//! median round trip. A connection left incomplete (no trailing `\n`) wedges
//! ironbar's entire IPC — every command after it hangs too, since ironbar's
//! accept loop is serial with no spawn. So every round trip here carries a
//! hard timeout and the connection is always closed, success or failure.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

use crate::vars::json_escape;

const ROUND_TRIP_TIMEOUT: Duration = Duration::from_secs(2);

pub struct IronbarIpc {
    sock: PathBuf,
    last_ino: Option<u64>,
}

impl IronbarIpc {
    pub fn new() -> Self {
        let sock = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join("ironbar-ipc.sock");
        Self { sock, last_ino: None }
    }

    /// True if the socket's inode changed since the last successful flush —
    /// the cheap, cooperation-free way to detect "ironbar restarted", since a
    /// fresh ironbar re-binds the path and starts from `ironvar_defaults`,
    /// not whatever we last sent it. One extra `stat(2)` per flush; far
    /// cheaper than a sentinel `var get` every flush would be.
    pub fn restarted(&mut self) -> bool {
        let ino = std::fs::metadata(&self.sock).ok().map(|m| m.ino());
        let changed = ino.is_some() && ino != self.last_ino;
        if ino.is_some() {
            self.last_ino = ino;
        }
        changed
    }

    /// Sends one `var set key value`. Connects, writes, reads, closes —
    /// always, regardless of outcome. Never leaves a half-written request on
    /// the wire (see module doc — that's the wedge hazard).
    pub async fn var_set(&self, key: &str, value: &str) -> io::Result<()> {
        let fut = async {
            let mut stream = UnixStream::connect(&self.sock).await?;
            let req = format!(
                "{{\"command\":\"var\",\"subcommand\":\"set\",\"key\":\"{}\",\"value\":\"{}\"}}\n",
                json_escape(key),
                json_escape(value),
            );
            stream.write_all(req.as_bytes()).await?;
            stream.shutdown().await.ok();
            let mut resp = Vec::with_capacity(32);
            stream.read_to_end(&mut resp).await?;
            if resp.starts_with(b"{\"type\":\"ok\"") {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "ironbar rejected var set: {}",
                    String::from_utf8_lossy(&resp)
                )))
            }
        };
        timeout(ROUND_TRIP_TIMEOUT, fut)
            .await
            .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "ironbar IPC round trip")))
    }
}
