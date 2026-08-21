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
        Self {
            sock,
            last_ino: None,
        }
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
        let req = format!(
            "{{\"command\":\"var\",\"subcommand\":\"set\",\"key\":\"{}\",\"value\":\"{}\"}}\n",
            json_escape(key),
            json_escape(value),
        );
        self.request(&req).await
    }

    /// Applies a workspace pill's CSS class delta: removes `old` (if any and
    /// non-empty), then adds `new` (if non-empty). `Class::Occupied` (T2,
    /// mango.rs) maps to `""` — the bare `#ws` look needs no class at all —
    /// so an empty side of the delta is simply skipped, not sent as a no-op
    /// round trip. Two connections worst case, one for the common case of
    /// leaving one class for another.
    pub async fn set_class(&self, module: &str, old: Option<&str>, new: &str) -> io::Result<()> {
        if let Some(old) = old.filter(|s| !s.is_empty()) {
            self.request(&style_cmd("remove_class", module, old))
                .await?;
        }
        if !new.is_empty() {
            self.request(&style_cmd("add_class", module, new)).await?;
        }
        Ok(())
    }

    /// One request/response round trip, per the module doc's rule: connect,
    /// write, read, close, always, with a hard timeout so a stuck peer can
    /// never wedge the caller.
    async fn request(&self, json: &str) -> io::Result<()> {
        let fut = async {
            let mut stream = UnixStream::connect(&self.sock).await?;
            stream.write_all(json.as_bytes()).await?;
            stream.shutdown().await.ok();
            let mut resp = Vec::with_capacity(32);
            stream.read_to_end(&mut resp).await?;
            if resp.starts_with(b"{\"type\":\"ok\"") {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "ironbar rejected request: {}",
                    String::from_utf8_lossy(&resp)
                )))
            }
        };
        timeout(ROUND_TRIP_TIMEOUT, fut).await.unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "ironbar IPC round trip",
            ))
        })
    }
}

/// `{"command":"style","subcommand":"add_class"|"remove_class","module_name":...,"name":...}\n`
/// — verified against the live binary's own serde field names (T0/T2 probe;
/// `ironbar style add-class <module> <class>` on the CLI maps to this).
fn style_cmd(subcommand: &str, module: &str, class: &str) -> String {
    format!(
        "{{\"command\":\"style\",\"subcommand\":\"{}\",\"module_name\":\"{}\",\"name\":\"{}\"}}\n",
        subcommand,
        json_escape(module),
        json_escape(class),
    )
}
