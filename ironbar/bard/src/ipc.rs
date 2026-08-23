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

    /// Opens `widget`'s popup on `bar`.
    ///
    /// **Sends `show_popup`, reverted from a `toggle_popup` substitution.**
    /// The earlier `"Module has no popup functionality"` error for every
    /// `custom`-type module (cpu, memory, docker, battery, volume,
    /// claudebar, clock, date, pomo, the workspace pills) was never a
    /// `show_popup` routing problem — it was `genconfig.rs` giving every
    /// popup-bearing module a `label` bar widget, and ironbar's own
    /// `src/modules/custom/label.rs`/`button.rs` (v0.19.0) show only
    /// `ButtonWidget` ever registers a popup anchor button
    /// (`context.popup_buttons`). A `label`-only module reaches
    /// `ipc/server/bar.rs::show_popup`'s `popup.buttons.first()` with an
    /// empty vec and correctly reports no popup functionality — the
    /// response was accurate. `toggle_popup` "worked" only because
    /// `BarCommandType::TogglePopup`'s handler calls `show_popup` and
    /// **discards its `Response`**, always returning `Response::Ok` — a
    /// false positive that hid the real bug and gave up `show_popup`'s
    /// genuine error reporting for nothing. Now that every popup-bearing
    /// module's bar widget is a `button` (see `genconfig.rs`), `show_popup`
    /// succeeds and correctly errors if it doesn't. Wire format read from
    /// ironbar's own source (`src/ipc/commands.rs`, GitHub, v0.19.0), not
    /// guessed: same shape as `style_cmd`'s `command":"style"` pair, one
    /// level up (`"command":"bar"`).
    pub async fn show_popup(&self, bar: &str, widget: &str) -> io::Result<()> {
        self.request(&bar_cmd("show_popup", bar, Some(widget)))
            .await
    }

    /// Closes whatever popup is open on `bar`, unconditionally — no
    /// `widget_name` field, matching ironbar's own `hide_popup` subcommand
    /// (it hides the bar's one open popup, not a specific widget's). This is
    /// what fixes T9's bluetooth-autohide gap for real: hover-exit always
    /// calls this, regardless of which popup GTK thinks is showing.
    pub async fn hide_popup(&self, bar: &str) -> io::Result<()> {
        self.request(&bar_cmd("hide_popup", bar, None)).await
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

/// `{"command":"bar","name":...,"subcommand":"show_popup"|"hide_popup"[,"widget_name":...]}\n`
/// — `show_popup` carries `widget_name`, `hide_popup` does not (ironbar's own
/// `src/ipc/commands.rs` types `hide_popup` with no widget field: it hides
/// whatever is open on the bar).
fn bar_cmd(subcommand: &str, bar: &str, widget: Option<&str>) -> String {
    match widget {
        Some(w) => format!(
            "{{\"command\":\"bar\",\"name\":\"{}\",\"subcommand\":\"{}\",\"widget_name\":\"{}\"}}\n",
            json_escape(bar),
            subcommand,
            json_escape(w),
        ),
        None => format!(
            "{{\"command\":\"bar\",\"name\":\"{}\",\"subcommand\":\"{}\"}}\n",
            json_escape(bar),
            subcommand,
        ),
    }
}
