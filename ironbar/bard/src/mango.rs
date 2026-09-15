//! The mango collector: nine workspace pills per monitor plus the focused-
//! window pill. `render()` and `window_text()`/`window_tip()` are exact
//! ports of src/waybar/scripts/workspace.sh's `render()` and
//! src/waybar/scripts/mango-window.sh's `JQ_FILTER` — see IRONBAR.md T2.
//!
//! Two long-lived `mmsg watch` children (`Watch`) supply complete state
//! with zero forks per event: `all-monitors` carries per-tag
//! is_active/is_urgent, active_tags (the overview sentinel) and
//! active_client in one document, so it also stands in for
//! mango-window.sh's third `focusing-client` stream.

use crate::cmd::CMD_TIMEOUT;
use crate::vars::Vars;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};
use tokio::time::timeout;

pub const TAG_COUNT: u64 = 9;

/// ironvar keys under this prefix never reach the wire as `var set` — the
/// flush loop (main.rs) routes them to the `style add-class`/`remove-class`
/// IPC command instead. Folding class delivery into `Vars`' existing dirty
/// tracking (rather than a parallel struct) means it inherits debounce
/// arming, ordering and ack-on-success/replay-on-failure for free.
pub const CLASS_PREFIX: &str = "@class/";

// ---------------------------------------------------------------- classes

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Overview,
    Urgent,
    Active,
    Empty,
    Occupied,
}

impl Class {
    /// CSS class sent for a numbered pill. `Overview` never reaches this —
    /// while a monitor is in overview its nine pills are `show_if`-hidden,
    /// so `apply()` skips their classes entirely rather than styling
    /// something nobody can see. `Occupied` is the bare `#ws` look (the
    /// pill's default appearance), so it needs no class at all.
    pub fn css(self) -> &'static str {
        match self {
            Class::Urgent => "urgent",
            Class::Active => "active",
            Class::Empty => "empty",
            Class::Occupied | Class::Overview => "",
        }
    }
}

// ---------------------------------------------------------------- render

pub struct Win<'a> {
    pub mark: char,
    pub appid: &'a str,
    pub title: &'a str,
}

/// Exact port of workspace.sh's `render()`. `mon == ""` means "first
/// monitor in the document" (workspace.sh:42); `None` means the tag or
/// monitor name matched nothing — workspace.sh renders nothing rather than
/// falling back to another screen's tags (its `HDMI-A-9` selftest case).
pub fn render<'a>(
    monitors: &'a Value,
    clients: &'a Value,
    mon: &str,
    tag: u64,
) -> Option<(Class, Vec<Win<'a>>)> {
    let mons = monitors.get("monitors")?.as_array()?;
    let name = if mon.is_empty() {
        mons.first()?.get("name")?.as_str()?
    } else {
        mon
    };
    let mo = mons
        .iter()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(name))?;
    let overview = is_overview(mo);
    let t = mo
        .get("tags")?
        .as_array()?
        .iter()
        .find(|t| t.get("index").and_then(Value::as_u64) == Some(tag))?;

    let wins: Vec<Win> = clients
        .get("clients")?
        .as_array()?
        .iter()
        .filter(|c| {
            c.get("monitor").and_then(Value::as_str) == Some(name)
                && (overview || tags_contain(c, tag))
        })
        .map(|c| Win {
            mark: mark_of(c),
            appid: c.get("appid").and_then(Value::as_str).unwrap_or("?"),
            title: truncate_chars(c.get("title").and_then(Value::as_str).unwrap_or(""), 44),
        })
        .collect();

    let class = if overview {
        Class::Overview
    } else if flag(t, "is_urgent") {
        Class::Urgent
    } else if flag(t, "is_active") {
        Class::Active
    } else if wins.is_empty() {
        Class::Empty
    } else {
        Class::Occupied
    };

    Some((class, wins))
}

/// `active_tags == [0]` exactly — tag indices are 1-based, so 0 is an
/// unambiguous sentinel for SUPER+space's all-tags overview.
fn is_overview(mo: &Value) -> bool {
    match mo.get("active_tags").and_then(Value::as_array) {
        Some(arr) => arr.len() == 1 && arr[0].as_u64() == Some(0),
        None => false,
    }
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn tags_contain(client: &Value, tag: u64) -> bool {
    client
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| a.iter().any(|v| v.as_u64() == Some(tag)))
        .unwrap_or(false)
}

fn mark_of(c: &Value) -> char {
    if flag(c, "is_urgent") {
        '!'
    } else if flag(c, "is_minimized") {
        '_'
    } else if flag(c, "is_focused") {
        '*'
    } else {
        ' '
    }
}

fn truncate_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

// ---------------------------------------------------------------- window pill

/// Port of mango-window.sh's `JQ_FILTER` bar text. `client` is
/// `all-monitors`'s `.monitors[] | select(.active) | .active_client`.
pub fn window_text(client: Option<&Value>) -> String {
    let appid = client
        .and_then(|c| c.get("appid"))
        .and_then(Value::as_str)
        .unwrap_or("Desktop");
    let title = client
        .and_then(|c| c.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("");
    // kitty/repo-title.py prefixes every kitty window title with its repo
    // label ("<repo> · <title>") — split it back out so the dim first line
    // reads the repo instead of the redundant "kitty" appid. Gated on
    // appid=="kitty": other apps' real titles can contain " · " too
    // (Firefox tabs do) and must not be split.
    let (line1, title) = if appid == "kitty" {
        title.split_once(" · ").unwrap_or((appid, title))
    } else {
        (appid, title)
    };
    let shown = truncate_ellipsis(title, 45);
    let mut out = format!(
        "<span size=\"small\" alpha=\"70%\">{}</span>",
        crate::tooltip::esc(line1)
    );
    if !shown.is_empty() {
        out.push('\n');
        out.push_str(&crate::tooltip::esc(&shown));
    }
    out
}

/// Raw, untruncated title (or appid) for the window pill's popup —
/// mango-window.sh's `tooltip` field, deliberately unescaped: its
/// `escape: false` module option passed the title through as Pango markup
/// verbatim, and the popup label is the same kind of surface.
pub fn window_tip(client: Option<&Value>) -> String {
    let appid = client
        .and_then(|c| c.get("appid"))
        .and_then(Value::as_str)
        .unwrap_or("Desktop");
    let title = client
        .and_then(|c| c.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if title.is_empty() {
        appid.to_string()
    } else {
        title.to_string()
    }
}

/// pub: music.rs reuses this verbatim for its own two-line pill (T28) —
/// same truncation rule as `window_text` above, so the two pills read as
/// one visual language.
pub fn truncate_ellipsis(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((idx, _)) => format!("{}…", &s[..idx]),
        None => s.to_string(),
    }
}

// ---------------------------------------------------------------- naming

/// Strips everything but ASCII alphanumerics — the CLI's own docs only
/// promise ironvar keys are "any alphanumeric ASCII string", so monitor
/// names (which carry `-`) are never used in a key directly.
pub fn slug(mon: &str) -> String {
    mon.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Slugs for `names`, in order. If any two collide (practically impossible,
/// silently wrong if unhandled), the WHOLE set falls back to `mon0`,
/// `mon1`, ... rather than patching just the collision — genconfig (which
/// computes this once at generation time) and `apply()` (which recomputes
/// it every document) must always agree on which scheme is in play.
pub fn slugs_for(names: &[String]) -> Vec<String> {
    let raw: Vec<String> = names.iter().map(|n| slug(n)).collect();
    let mut seen = std::collections::HashSet::new();
    let collision = raw.iter().any(|s| !seen.insert(s.clone()));
    if collision {
        (0..names.len()).map(|i| format!("mon{i}")).collect()
    } else {
        raw
    }
}

pub fn ws_module(mon: &str, tag: u64) -> String {
    format!("ws-{mon}-{tag}")
}

pub fn ws_module_ov(mon: &str) -> String {
    format!("ws-{mon}-ov")
}

fn class_key(mon: &str, tag: u64) -> String {
    format!("{CLASS_PREFIX}{}", ws_module(mon, tag))
}

/// Cap on the dots drawn under a workspace pill's number — see [`tag_label`].
/// A ring/dot count past this stops reading as countable at pill size, same
/// reasoning the old dashed-ring `count_class` used for its own "dash-4plus"
/// bucket.
const MAX_DOTS: usize = 5;

/// Two-line Pango label for a workspace pill: the tag number, then a small
/// centred dot row underneath, one dot per window, capped at [`MAX_DOTS`].
/// Replaces the T9 dashed-ring `count_class`/`count_class_key` pair — a
/// literal dot count reads at a glance without needing a
/// `@class/<module>#count` CSS slot at all, so this rides the pill's own
/// `bar` label var instead of a class.
///
/// T16: the dot row is now emitted even at zero windows (one dot,
/// `alpha="1%"`, effectively invisible — Pango rejects a literal `0%`)
/// instead of collapsing to a bare one-line number. A tag with no windows
/// used to render a shorter block than a tag with windows, so GTK centred
/// each at a different height inside the pill and no single `.ws` padding
/// value in style.css could line both up with the clock text next to them.
/// A constant two-line block makes the padding correction constant too.
///
/// T20: three lines now, not two. The two-line block put the number above
/// the vertical centre of the pill's oval — the dot row's height hung off
/// the bottom only, and style.css compensated with an asymmetric
/// screenshot-tuned top padding that centred the block against its
/// neighbours but never centred the number inside its own oval. A mirror
/// line above the number (same glyphs, always `alpha="1%"`) makes the
/// block symmetric: the number is the block's exact centre, so one plain
/// symmetric `.ws` padding centres it in the oval and against the row at
/// the same time. No tuned constant left to drift.
pub fn tag_label(n: u64, win_count: usize) -> String {
    let dots = win_count.min(MAX_DOTS);
    let glyphs: String = "\u{2022}".repeat(dots.max(1));
    let alpha = if dots == 0 { "1%" } else { "45%" };
    format!(
        "<span size=\"38%\" alpha=\"1%\">{glyphs}</span>\n{n}\n<span size=\"38%\" alpha=\"{alpha}\">{glyphs}</span>"
    )
}

/// Static label for the `.ws-overview` pill — same three-line shape as
/// [`tag_label`] (invisible mirror line, content, invisible mirror line),
/// so the overview pill's height matches a numbered pill's exactly. Without
/// this, a bare one-line "overview" label made GTK size the overview oval
/// shorter than the pills it replaces, since ovals in the same row don't
/// otherwise share a forced height. Both mirror lines are always invisible
/// (`alpha="1%"`, `win_count = 0`'s case in `tag_label`) — the overview
/// pill has no window count of its own to show a real dot row for.
pub fn overview_label() -> String {
    let dot = "\u{2022}";
    format!("<span size=\"38%\" alpha=\"1%\">{dot}</span>\noverview\n<span size=\"38%\" alpha=\"1%\">{dot}</span>")
}

pub fn var_tags(slug: &str) -> String {
    format!("ws_{slug}_tags")
}

pub fn var_ov(slug: &str) -> String {
    format!("ws_{slug}_ov")
}

pub fn var_tip(slug: &str, tag: u64) -> String {
    format!("ws_{slug}_{tag}_tip")
}

/// The pill's own two-line `bar` label var — see [`tag_label`]. Named next
/// to `var_tip` since both are set from the same `apply()` loop and both
/// need a matching `ironvar_defaults` entry in genconfig.rs.
pub fn var_lbl(slug: &str, tag: u64) -> String {
    format!("ws_{slug}_{tag}_lbl")
}

// ---------------------------------------------------------------- watch

const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// A restartable `mmsg watch <topic>` child, exposed as one cancel-safe
/// `next_doc()`. Deliberately not an `AsRawFd` collector like clock.rs or
/// powermode.rs — it owns a child process, not a raw fd, so it gets its own
/// honest shape in the select! loop instead of forcing that contract.
pub struct Watch {
    topic: &'static str,
    lines: Option<Lines<BufReader<ChildStdout>>>,
    child: Option<Child>,
    /// The initial `mmsg get` snapshot, returned once before the live
    /// stream. `mmsg watch` never emits a snapshot of its own.
    pending: Option<String>,
    backoff: Duration,
}

impl Watch {
    pub fn new(topic: &'static str) -> Self {
        Self {
            topic,
            lines: None,
            child: None,
            pending: None,
            backoff: BACKOFF_START,
        }
    }

    /// Next document, in the order mango emitted it. Loops internally over
    /// spawn / respawn-after-EOF / backoff, so callers only ever see a
    /// line and this never returns an error. `Lines::next_line()` is
    /// documented cancel-safe, which matters here: `select!` drops the
    /// losing arm's future every iteration, and a non-cancel-safe read
    /// (e.g. `read_line` into a caller-owned buffer) would silently
    /// corrupt a document split across two reads.
    pub async fn next_doc(&mut self) -> String {
        loop {
            if let Some(doc) = self.pending.take() {
                return doc;
            }
            if self.lines.is_none() {
                if let Err(e) = self.spawn().await {
                    eprintln!("mango-bard: mmsg watch {}: {e}", self.topic);
                    tokio::time::sleep(self.backoff).await;
                    self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                    continue;
                }
                self.backoff = BACKOFF_START;
                continue; // pending now holds the get snapshot (if any)
            }
            match self
                .lines
                .as_mut()
                .expect("checked above")
                .next_line()
                .await
            {
                Ok(Some(line)) => return line,
                Ok(None) | Err(_) => {
                    // EOF or read error: reap, drop, respawn (which re-runs
                    // the initial `get` — that's what resyncs us after
                    // mango itself restarts).
                    self.lines = None;
                    if let Some(mut child) = self.child.take() {
                        let _ = child.start_kill();
                    }
                }
            }
        }
    }

    /// Spawns `mmsg watch <topic>` FIRST — before the initial `get` — so
    /// any event landing between the get returning and the watch
    /// connecting queues in the pipe instead of being lost (both shell
    /// ports pair `get`-then-`watch`, which leaves exactly this race open).
    ///
    /// ponytail: `.output()`'s internal child isn't `kill_on_drop`, so a
    /// `next_doc()` cancelled during this one `mmsg get` could in theory
    /// leave it running — accepted, since `mmsg get` is short-lived and
    /// self-terminating either way, unlike the `watch` child this exists to
    /// avoid leaking.
    async fn spawn(&mut self) -> std::io::Result<()> {
        let mut child = Command::new("mmsg")
            .args(["watch", self.topic])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout requested above");
        self.lines = Some(BufReader::new(stdout).lines());
        self.child = Some(child);

        let get = timeout(
            CMD_TIMEOUT,
            Command::new("mmsg")
                .args(["get", self.topic])
                .stderr(Stdio::null())
                .output(),
        )
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "mmsg get"))??;
        let snapshot = String::from_utf8_lossy(&get.stdout).trim().to_string();
        self.pending = if snapshot.is_empty() {
            None
        } else {
            Some(snapshot)
        };
        Ok(())
    }

    /// A silent `mmsg watch` never writes, so it never takes SIGPIPE and
    /// leaks forever (the failure mango-window.sh's header comment
    /// documents at length) — `kill_on_drop` alone is not enough because
    /// nothing here drops the `Watch` before process exit; SIGTERM handling
    /// must call this explicitly.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        self.lines = None;
        self.pending = None;
    }
}

// ---------------------------------------------------------------- Mango

pub struct Mango {
    pub monitors: Watch,
    pub clients: Watch,
    last_monitors_raw: Option<String>,
    last_clients_raw: Option<String>,
    monitors_doc: Value,
    clients_doc: Value,
    /// `(name, slug)` pairs `apply()` emitted keys for last time it ran —
    /// compared against the live monitor list on every call so a monitor
    /// mango has torn down (a destroyed virtual output) gets its keys
    /// dropped via `Vars::forget` instead of retried forever. See that
    /// prune loop at the top of `apply()`.
    last_mons: Vec<(String, String)>,
}

impl Mango {
    pub fn new() -> Self {
        Self {
            monitors: Watch::new("all-monitors"),
            clients: Watch::new("all-clients"),
            last_monitors_raw: None,
            last_clients_raw: None,
            monitors_doc: Value::Null,
            clients_doc: Value::Null,
            last_mons: Vec::new(),
        }
    }

    /// Parses and stores `line` as the current `all-monitors` document.
    /// Returns false if it is byte-identical to the previous line — layer
    /// one of the eco-invariant dedup (IRONBAR.md T2): `all-monitors` also
    /// carries focus/keymode churn that changes nothing this bar renders
    /// (bars.sh's own comment: it "fires constantly under sloppyfocus"), so
    /// a memcmp here means that churn never reaches a parse, let alone a
    /// flush.
    pub fn ingest_monitors(&mut self, line: &str) -> bool {
        if self.last_monitors_raw.as_deref() == Some(line) {
            return false;
        }
        self.last_monitors_raw = Some(line.to_string());
        match serde_json::from_str(line) {
            Ok(v) => {
                self.monitors_doc = v;
                true
            }
            Err(e) => {
                eprintln!("mango-bard: bad all-monitors JSON: {e}");
                false
            }
        }
    }

    pub fn ingest_clients(&mut self, line: &str) -> bool {
        if self.last_clients_raw.as_deref() == Some(line) {
            return false;
        }
        self.last_clients_raw = Some(line.to_string());
        match serde_json::from_str(line) {
            Ok(v) => {
                self.clients_doc = v;
                true
            }
            Err(e) => {
                eprintln!("mango-bard: bad all-clients JSON: {e}");
                false
            }
        }
    }

    /// Re-derives every ironvar and pill class from the currently cached
    /// documents. Idempotent by construction — `Vars::set` only dirties a
    /// key whose value actually changed (see `apply_is_idempotent` below),
    /// which is layer two of the eco-invariant dedup.
    pub fn apply(&mut self, vars: &mut Vars) {
        let Some(mons) = self.monitors_doc.get("monitors").and_then(Value::as_array) else {
            return;
        };
        // A disabled output (width 0 — DPMS/only_sleep, or never enabled)
        // gets no ironbar bar, so any class/var send naming its modules
        // comes back "Module not found". Drop it here, before `names` is
        // built, so the prune loop below sees it as gone (same path as a
        // destroyed virtual output) instead of retrying forever — width 0
        // never disappears from `all-monitors` on its own, unlike a real
        // unplug, so without this filter it never reaches `names.contains`
        // returning false and is never forgotten.
        //
        // `!= Some(0)`, not `unwrap_or(0) > 0`: a real `mmsg get
        // all-monitors` always carries `width`, but the hand-written test
        // fixtures below don't bother with it — an absent field must read
        // as "unknown, assume enabled", not "explicitly zero".
        let mons: Vec<&Value> = mons
            .iter()
            .filter(|m| m.get("width").and_then(Value::as_u64) != Some(0))
            .collect();

        let active_client = mons
            .iter()
            .find(|m| m.get("active").and_then(Value::as_bool) == Some(true))
            .and_then(|m| m.get("active_client"));
        vars.set("win_text", window_text(active_client));
        vars.set("win_tip", window_tip(active_client));

        let names: Vec<String> = mons
            .iter()
            .filter_map(|m| m.get("name").and_then(Value::as_str))
            .map(String::from)
            .collect();
        let slugs = slugs_for(&names);

        // A monitor from last run that isn't in the live list any more (a
        // virtual output mango has destroyed, or — in principle — a
        // physical one unplugged) has its keys dropped outright, not left
        // to retry: they can never ACK again. Keyed off the live monitor
        // list, not off which keys got set this pass — a monitor in
        // overview mode `continue`s before its tag keys are touched, and
        // must not be mistaken for a vanished one.
        for (name, slug) in &self.last_mons {
            if names.contains(name) {
                continue;
            }
            vars.forget(&var_tags(slug));
            vars.forget(&var_ov(slug));
            for tag in 1..=TAG_COUNT {
                vars.forget(&class_key(name, tag));
                vars.forget(&var_lbl(slug, tag));
                vars.forget(&var_tip(slug, tag));
            }
        }
        self.last_mons = names.iter().cloned().zip(slugs.iter().cloned()).collect();

        for (mo, slug) in mons.iter().zip(&slugs) {
            let Some(name) = mo.get("name").and_then(Value::as_str) else {
                continue;
            };
            let overview = is_overview(mo);
            vars.set(&var_tags(slug), if overview { "false" } else { "true" });
            vars.set(&var_ov(slug), if overview { "true" } else { "false" });

            if overview {
                // The nine pills are show_if-hidden; their classes don't
                // matter while nobody can see them, so skip nine IPC calls.
                continue;
            }
            for tag in 1..=TAG_COUNT {
                let Some((class, wins)) = render(&self.monitors_doc, &self.clients_doc, name, tag)
                else {
                    continue;
                };
                vars.set(&class_key(name, tag), class.css());
                vars.set(&var_lbl(slug, tag), tag_label(tag, wins.len()));
                // The popup body already *is* the window list — a
                // "right-click: window list" footer told the user to do the
                // thing they were already looking at, so it's dropped
                // rather than ported through set_tip's HINTS mechanism.
                vars.set(&var_tip(slug, tag), crate::tooltip::window_list(&wins));
            }
        }
    }
}

impl Default for Mango {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures copied verbatim from src/waybar/scripts/workspace.sh:112-126
    // and :169-171, so a future diff against the shell source stays trivial.
    const TAGS: &str = r#"{"monitors":[{"name":"eDP-1","active_tags":[2],"tags":[
		{"index":1,"is_active":false,"is_urgent":false,"client_count":2},
		{"index":2,"is_active":true,"is_urgent":false,"client_count":0},
		{"index":3,"is_active":false,"is_urgent":true,"client_count":1},
		{"index":4,"is_active":false,"is_urgent":false,"client_count":0}]},
		{"name":"DP-1","active_tags":[1],"tags":[
		{"index":1,"is_active":true,"is_urgent":false,"client_count":1},
		{"index":2,"is_active":false,"is_urgent":false,"client_count":0},
		{"index":3,"is_active":false,"is_urgent":false,"client_count":0},
		{"index":4,"is_active":false,"is_urgent":false,"client_count":0}]}]}"#;
    const CLIENTS: &str = r#"{"clients":[
		{"appid":"kitty","title":"vim x & y <z>","monitor":"eDP-1","tags":[1],"is_focused":true,"is_urgent":false,"is_minimized":false},
		{"appid":"firefox","title":"news","monitor":"eDP-1","tags":[1],"is_focused":false,"is_urgent":false,"is_minimized":true},
		{"appid":"slack","title":"ping","monitor":"eDP-1","tags":[3],"is_focused":false,"is_urgent":true,"is_minimized":false},
		{"appid":"mpv","title":"film","monitor":"DP-1","tags":[1],"is_focused":true,"is_urgent":false,"is_minimized":false}]}"#;
    const OVTAGS: &str = r#"{"monitors":[{"name":"eDP-1","active_tags":[0],"tags":[
		{"index":1,"is_active":true,"is_urgent":false,"client_count":1},
		{"index":2,"is_active":true,"is_urgent":true,"client_count":0}]}]}"#;

    fn feed(n: u64, mon: &str) -> Option<(Class, Vec<Win<'static>>)> {
        let monitors: Value = serde_json::from_str(TAGS).unwrap();
        let clients: Value = serde_json::from_str(CLIENTS).unwrap();
        // Leak so the borrowed Win<'a> can outlive this helper — fine in tests.
        let monitors: &'static Value = Box::leak(Box::new(monitors));
        let clients: &'static Value = Box::leak(Box::new(clients));
        render(monitors, clients, mon, n)
    }

    fn ovfeed(n: u64) -> Option<(Class, Vec<Win<'static>>)> {
        let monitors: Value = serde_json::from_str(OVTAGS).unwrap();
        let clients: Value = serde_json::from_str(CLIENTS).unwrap();
        let monitors: &'static Value = Box::leak(Box::new(monitors));
        let clients: &'static Value = Box::leak(Box::new(clients));
        render(monitors, clients, n_mon(), n)
    }

    fn n_mon() -> &'static str {
        "eDP-1"
    }

    #[test]
    fn class_precedence_matches_workspace_sh() {
        assert_eq!(feed(1, "").unwrap().0, Class::Occupied);
        assert_eq!(feed(2, "").unwrap().0, Class::Active);
        assert_eq!(feed(3, "").unwrap().0, Class::Urgent);
        assert_eq!(feed(4, "").unwrap().0, Class::Empty);
    }

    #[test]
    fn tag_label_dots_cap_at_five_and_stay_invisible_at_zero() {
        // T16/T20: zero windows still renders a full block (one dot,
        // alpha 1%) so the block height is constant regardless of
        // window count, and a mirror line above the number keeps the
        // number at the block's centre — see tag_label's own doc comment.
        assert_eq!(
            tag_label(3, 0),
            "<span size=\"38%\" alpha=\"1%\">\u{2022}</span>\n3\n<span size=\"38%\" alpha=\"1%\">\u{2022}</span>"
        );
        assert_eq!(
            tag_label(3, 1),
            "<span size=\"38%\" alpha=\"1%\">\u{2022}</span>\n3\n<span size=\"38%\" alpha=\"45%\">\u{2022}</span>"
        );
        assert_eq!(
            tag_label(3, 9),
            format!(
                "<span size=\"38%\" alpha=\"1%\">{d}</span>\n3\n<span size=\"38%\" alpha=\"45%\">{d}</span>",
                d = "\u{2022}".repeat(5)
            )
        );
    }

    #[test]
    fn overview_label_matches_tag_labels_line_count_for_height_parity() {
        // T-next (item 5): `.ws-overview` must be the same height as a `.ws`
        // pill, or the overview oval and the numbered pills it replaces
        // don't line up. Same three-line shape as tag_label's zero-window
        // case (one invisible dot both mirror lines), just "overview" where
        // the number goes.
        assert_eq!(
            overview_label(),
            "<span size=\"38%\" alpha=\"1%\">\u{2022}</span>\noverview\n<span size=\"38%\" alpha=\"1%\">\u{2022}</span>"
        );
        assert_eq!(
            overview_label().matches('\n').count(),
            tag_label(1, 0).matches('\n').count()
        );
    }

    #[test]
    fn tag_one_dots_match_its_window_count() {
        // T9/T-next: feed(1, "") has 2 windows (kitty, firefox) per
        // tag_one_window_rows_match above.
        let (_, wins) = feed(1, "").unwrap();
        assert_eq!(
            tag_label(1, wins.len()),
            format!(
                "<span size=\"38%\" alpha=\"1%\">{d}</span>\n1\n<span size=\"38%\" alpha=\"45%\">{d}</span>",
                d = "\u{2022}\u{2022}"
            )
        );
    }

    #[test]
    fn active_tag_with_clients_stays_active_not_occupied() {
        assert_eq!(feed(2, "").unwrap().1.len(), 0);
    }

    #[test]
    fn tag_one_window_rows_match() {
        let (_, wins) = feed(1, "").unwrap();
        assert_eq!(wins.len(), 2);
        assert_eq!(wins[0].mark, '*');
        assert_eq!(wins[0].appid, "kitty");
        assert_eq!(wins[0].title, "vim x & y <z>");
        assert_eq!(wins[1].mark, '_');
        assert_eq!(wins[1].appid, "firefox");
    }

    #[test]
    fn urgent_mark_matches() {
        let (_, wins) = feed(3, "").unwrap();
        assert_eq!(wins[0].mark, '!');
        assert_eq!(wins[0].appid, "slack");
    }

    #[test]
    fn per_monitor_isolation() {
        assert_eq!(feed(1, "eDP-1").unwrap().0, Class::Occupied);
        assert_eq!(feed(1, "DP-1").unwrap().0, Class::Active);
        assert_eq!(feed(3, "DP-1").unwrap().0, Class::Empty); // urgency must not cross monitors
        assert!(feed(1, "DP-1").unwrap().1.iter().any(|w| w.appid == "mpv"));
        assert!(!feed(1, "DP-1")
            .unwrap()
            .1
            .iter()
            .any(|w| w.appid == "kitty"));
        assert!(!feed(1, "eDP-1").unwrap().1.iter().any(|w| w.appid == "mpv"));
    }

    #[test]
    fn empty_monitor_arg_is_first_monitor_in_document() {
        assert_eq!(
            feed(1, "").unwrap().1.len(),
            feed(1, "eDP-1").unwrap().1.len()
        );
    }

    #[test]
    fn unknown_monitor_renders_nothing() {
        assert!(feed(1, "HDMI-A-9").is_none());
    }

    #[test]
    fn overview_outranks_urgent_and_lists_whole_monitor() {
        assert_eq!(ovfeed(1).unwrap().0, Class::Overview);
        assert_eq!(ovfeed(2).unwrap().0, Class::Overview);
        // workspace.sh's own assertion is `wc -l == 4` on class-line + rows;
        // as a row count (no class line here) that is 3 — kitty, firefox,
        // slack, all on eDP-1. mpv is excluded: it's on DP-1.
        let (_, wins) = ovfeed(1).unwrap();
        assert_eq!(wins.len(), 3); // per-tag filter dropped, per-monitor filter kept
        assert!(wins.iter().any(|w| w.appid == "slack"));
    }

    #[test]
    fn slug_strips_non_alphanumeric() {
        assert_eq!(slug("eDP-1"), "eDP1");
        assert_eq!(slug("HDMI-A-1"), "HDMIA1");
    }

    #[test]
    fn colliding_slugs_fall_back_to_positional_names() {
        let names = vec!["eDP-1".to_string(), "eDP1".to_string()];
        assert_eq!(
            slugs_for(&names),
            vec!["mon0".to_string(), "mon1".to_string()]
        );
    }

    #[test]
    fn apply_is_idempotent() {
        let mut m = Mango::new();
        m.ingest_monitors(TAGS);
        m.ingest_clients(CLIENTS);
        let mut vars = Vars::new();
        m.apply(&mut vars);
        assert!(vars.has_dirty());
        // Drain the dirty set exactly the way the real flush loop's
        // ack() does, then apply the SAME documents again.
        while let Some((k, _)) = vars
            .peek_dirty()
            .map(|(k, v)| (k.to_string(), v.to_string()))
        {
            vars.ack(&k);
        }
        m.apply(&mut vars);
        assert!(
            !vars.has_dirty(),
            "re-applying unchanged documents must dirty nothing"
        );
    }

    #[test]
    fn apply_forgets_keys_for_a_monitor_that_disappears() {
        // Regression test for T-freeze-2026-08-29: a virtual output's
        // `ws-HEADLESS-*` keys kept retrying every 5s for 17+ minutes after
        // `remote.sh --toggle` tore it down. `apply()` must drop them, not
        // leave them for `Vars`' cooldown to keep replaying forever.
        const ONE_MON: &str = r#"{"monitors":[{"name":"eDP-1","active_tags":[2],"tags":[
			{"index":1,"is_active":false,"is_urgent":false,"client_count":2},
			{"index":2,"is_active":true,"is_urgent":false,"client_count":0}]}]}"#;

        let mut m = Mango::new();
        let mut vars = Vars::new();
        m.ingest_monitors(TAGS); // eDP-1 + DP-1
        m.ingest_clients(CLIENTS);
        m.apply(&mut vars);
        while let Some((k, _)) = vars
            .peek_dirty()
            .map(|(k, v)| (k.to_string(), v.to_string()))
        {
            vars.ack(&k);
        }
        assert!(vars.live_value(&class_key("DP-1", 1)).is_some());

        m.ingest_monitors(ONE_MON); // DP-1 is gone
        m.apply(&mut vars);

        assert_eq!(
            vars.live_value(&class_key("DP-1", 1)),
            None,
            "DP-1's class key must be dropped, not just cooling down"
        );
        assert!(
            !vars.has_dirty(),
            "a forgotten monitor's keys must not sit in `dirty` either, and \
             eDP-1's own unchanged values must not have been re-dirtied"
        );
    }

    #[test]
    fn window_text_formats_appid_and_title() {
        let none = window_text(None);
        assert!(none.contains("Desktop"));
        assert!(!none.contains('\n'));

        let c: Value = serde_json::from_str(r#"{"appid":"kitty","title":""}"#).unwrap();
        assert!(!window_text(Some(&c)).contains('\n'));

        let long_title = "x".repeat(46);
        let c: Value =
            serde_json::from_str(&format!(r#"{{"appid":"a","title":"{long_title}"}}"#)).unwrap();
        let text = window_text(Some(&c));
        assert!(text.contains(&"x".repeat(45)));
        assert!(text.contains('…'));
        assert!(!text.contains(&"x".repeat(46)));

        let c: Value = serde_json::from_str(r#"{"appid":"a&b","title":"x<y"}"#).unwrap();
        let text = window_text(Some(&c));
        assert!(text.contains("a&amp;b"));
        assert!(text.contains("x&lt;y"));

        // kitty/repo-title.py prefixes kitty windows with "<repo> · title" —
        // split back into two dim/title lines instead of showing raw appid.
        let c: Value =
            serde_json::from_str(r#"{"appid":"kitty","title":"mango-dotfiles/ironbar · task"}"#)
                .unwrap();
        let text = window_text(Some(&c));
        assert!(
            text.starts_with("<span size=\"small\" alpha=\"70%\">mango-dotfiles/ironbar</span>")
        );
        assert!(text.ends_with("task"));

        // Non-kitty apps keep their raw appid even if the title happens to
        // contain " · " — Firefox tab titles do, and must not be split.
        let c: Value = serde_json::from_str(r#"{"appid":"firefox","title":"a · b"}"#).unwrap();
        let text = window_text(Some(&c));
        assert!(text.contains("firefox"));
        assert!(text.contains("a · b"));
    }

    #[test]
    fn window_tip_is_raw_and_untruncated() {
        assert_eq!(window_tip(None), "Desktop");
        let c: Value =
            serde_json::from_str(r#"{"appid":"a","title":"raw & <unescaped>"}"#).unwrap();
        assert_eq!(window_tip(Some(&c)), "raw & <unescaped>");
    }
}
