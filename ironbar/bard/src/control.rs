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

/// `hover enter <bar> <widget>` / `hover exit <bar> <widget>` — the two
/// verbs a hover-eligible module's `on_mouse_enter`/`on_mouse_exit` send.
/// Only two verbs exist, so a plain enum (not a free-text string, unlike
/// `Pomo`'s verb) keeps `main.rs`'s match exhaustive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HoverEvent {
    Enter,
    Exit,
}

/// `hover hold <bar>` / `hover release <bar>` — the popup's own *content
/// box* `on_mouse_enter`/`on_mouse_exit` (T-popup-hold: genconfig.rs's
/// `popup()` wires the outer vertical box to these, distinct from
/// `HoverEvent`'s pill-level enter/exit). A separate type rather than
/// folding into `HoverEvent` with an `Option<String>` widget: the popup box
/// has no module name of its own to report, so a shared `Line::Hover(_,
/// bar, Option<widget>)` shape would let an `Enter`/`Exit` with `widget:
/// None` type-check even though that combination can never legally occur —
/// two distinct `Line` variants make that state unrepresentable instead of
/// relying on an invariant enforced only by convention at each call site.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupHoverEvent {
    Hold,
    Release,
}

pub enum Line {
    Refresh(String),
    /// `pomo <verb> [arg]` — the arg (a task name, a resume line, a mute
    /// duration...) is everything after the verb, unsplit, since a task
    /// name is free text that may itself contain spaces.
    Pomo(String, Option<String>),
    /// `(event, bar, widget)` — bar and widget names are both ironbar
    /// identifiers (alphanumeric plus `-`, see `mango::ws_module`), never
    /// free text, so a plain three-way space split is exact — unlike
    /// `Pomo`'s arg, neither field can itself contain a space.
    Hover(HoverEvent, String, String),
    /// `(event, bar)` — see [`PopupHoverEvent`]'s own doc comment for why
    /// this isn't folded into `Hover` above.
    HoverPopup(PopupHoverEvent, String),
    Ping,
    Stats,
    Unknown,
}

pub fn parse(line: &str) -> Line {
    let line = line.trim();
    if let Some(topic) = line.strip_prefix("refresh ") {
        Line::Refresh(topic.trim().to_string())
    } else if let Some(rest) = line.strip_prefix("pomo ") {
        let rest = rest.trim();
        match rest.split_once(' ') {
            Some((verb, arg)) => Line::Pomo(verb.to_string(), Some(arg.trim().to_string())),
            None => Line::Pomo(rest.to_string(), None),
        }
    } else if let Some(rest) = line.strip_prefix("hover ") {
        // Enter/exit: exactly 3 tokens (verb, bar, widget). Hold/release:
        // exactly 2 (verb, bar) — the popup content box has no widget name
        // of its own. Bar/widget names never carry whitespace (they're
        // ironbar identifiers, see `mango::ws_module`), so an exact split —
        // not `Pomo`'s free-text tail — is correct, and a malformed line
        // (wrong token count) falls through to Unknown rather than silently
        // swallowing a stray trailing token.
        match rest.split_whitespace().collect::<Vec<&str>>().as_slice() {
            ["enter", bar, widget] => {
                Line::Hover(HoverEvent::Enter, bar.to_string(), widget.to_string())
            }
            ["exit", bar, widget] => {
                Line::Hover(HoverEvent::Exit, bar.to_string(), widget.to_string())
            }
            ["hold", bar] => Line::HoverPopup(PopupHoverEvent::Hold, bar.to_string()),
            ["release", bar] => Line::HoverPopup(PopupHoverEvent::Release, bar.to_string()),
            _ => Line::Unknown,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_enter_parses_bar_and_widget() {
        match parse("hover enter bar-eDP-1 cpu") {
            Line::Hover(HoverEvent::Enter, bar, widget) => {
                assert_eq!(bar, "bar-eDP-1");
                assert_eq!(widget, "cpu");
            }
            _ => panic!("expected Line::Hover(Enter, ...)"),
        }
    }

    #[test]
    fn hover_exit_parses_bar_and_widget() {
        match parse("hover exit bar-default bluetooth") {
            Line::Hover(HoverEvent::Exit, bar, widget) => {
                assert_eq!(bar, "bar-default");
                assert_eq!(widget, "bluetooth");
            }
            _ => panic!("expected Line::Hover(Exit, ...)"),
        }
    }

    #[test]
    fn hover_hold_parses_bar_only() {
        match parse("hover hold bar-eDP-1") {
            Line::HoverPopup(PopupHoverEvent::Hold, bar) => {
                assert_eq!(bar, "bar-eDP-1");
            }
            _ => panic!("expected Line::HoverPopup(Hold, ...)"),
        }
    }

    #[test]
    fn hover_release_parses_bar_only() {
        match parse("hover release bar-eDP-1") {
            Line::HoverPopup(PopupHoverEvent::Release, bar) => {
                assert_eq!(bar, "bar-eDP-1");
            }
            _ => panic!("expected Line::HoverPopup(Release, ...)"),
        }
    }

    #[test]
    fn hover_hold_rejects_a_widget_token() {
        // Hold/release take only a bar — a third token is the enter/exit
        // shape, not a malformed hold/release, so it must not silently
        // parse as either.
        assert!(matches!(
            parse("hover hold bar-eDP-1 cpu"),
            Line::Unknown
        ));
    }

    #[test]
    fn hover_with_wrong_arity_is_unknown() {
        assert!(matches!(parse("hover enter bar-eDP-1"), Line::Unknown));
        assert!(matches!(
            parse("hover enter bar-eDP-1 cpu extra"),
            Line::Unknown
        ));
        assert!(matches!(
            parse("hover sideways bar-eDP-1 cpu"),
            Line::Unknown
        ));
    }
}
