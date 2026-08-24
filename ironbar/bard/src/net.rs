//! The network collector: wifi/eth/netsec pills plus the busy spinner.
//! Ports src/ironbar/scripts/net.sh (state machine, RSSI, icons) and
//! src/waybar/scripts/net-watch.sh (nmcli monitor line classification) — see
//! IRONBAR.md T3 for the design (no zbus, RSSI on the gated minute tick,
//! CSS-animated spinner) and the "Corrections to earlier sections" T3 entry
//! for why the original NM D-Bus sketch was dropped.
//!
//! Only bar text is ported here (`wifi_text`/`eth_text`/`sec_text` — plain
//! icon + short readout). The rich pango tooltip bodies (`sec_emit`'s
//! "Routes"/"Tunnels"/"Resolvers" sections, `wifi_emit`'s "Radio"/
//! "Throughput" sections) are T7's job (IRONBAR.md "tooltips/popups
//! parity") — T3's genconfig modules have no `popup` field, matching that
//! scope line.
//!
//! `regrade()` is the only thing that forks. Two long-lived children
//! (`nmcli monitor`, `ip -o monitor route`) plus the clock tick, a powermode
//! change, and the control socket all feed it through a 150ms debounce
//! (main.rs's `regrade_due`) — the same constant, and the same "coalesce a
//! burst, fire once it goes quiet" shape, as net-watch.sh's own `DEBOUNCE`.

use crate::mango::CLASS_PREFIX;
use crate::powermode;
use crate::tooltip::{
    bad, barico, barico_label, dim, esc, good, human, mono, row, sect, set_titled, warn,
};
use crate::vars::Vars;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

// ------------------------------------------------------------------ icons
//
// Nerd Font PUA codepoints — invisible in an editor, so named here instead,
// exactly as net.sh:43-51 does with shell functions.
//
// T8b: every icon below was originally a Material Symbols Rounded
// codepoint (net.sh's own comments, still shown per-constant). GTK4 cannot
// correctly rasterize Material Symbols Rounded's variable font on this
// system, so all are replaced with JetBrainsMono Nerd Font equivalents.
// See IRONBAR.md's T8b entry for the full investigation — including why
// pango-view rendering a candidate correctly, or a codepoint being a
// genuine Nerd Font PUA glyph, is NOT sufficient evidence either: several
// first-choice Nerd Font replacements (e.g. a literal "login" glyph for
// IC_PORTAL, a "warning" triangle for IC_CONFLICT) also rendered as wrong
// CJK tofu live in this ironbar/GTK4 build and had to be swapped again —
// this was the T17 "icon font never reached any label" CSS bug, not a
// property of these codepoints (T17's own entry has the mechanism). T18
// re-ran the same swap now that the CSS is fixed and gave every state its
// own glyph instead of the three T8b reuses this comment used to justify:
// IC_LOCK was `\u{f084}`, nf-fa-key — the "secure" state drew a KEY, not
// a lock, since T8b. IC_OPEN and IC_EXPOSED shared one open-lock glyph;
// IC_OFFLINE and IC_CONFLICT both shared IC_ETHOFF's crossed-circle. All
// nine below are distinct Plane-15 `nf-md-*` glyphs, each rendered with
// `pango-view --font="JetBrainsMono Nerd Font Propo"` before wiring in
// (Plane-15 PUA has no competing CJK claimant — same reasoning as T16's
// battery-leaf fix). `DnsLeak` still shares `IC_LOCK` with `Secure`
// deliberately: `.netsec.dnsleak`'s caution color already separates them
// and a fourth lock variant isn't worth a tenth constant.
pub const IC_OFFLINE: char = '\u{f0164}'; // md-cloud_off_outline
pub const IC_PORTAL: char = '\u{f0342}'; // md-login
pub const IC_OPEN: char = '\u{f0fc6}'; // md-lock_open_variant
pub const IC_EXPOSED: char = '\u{f099c}'; // md-shield_off_outline
pub const IC_LOCK: char = '\u{f033e}'; // md-lock (was f084, nf-fa-key — wrong glyph)
pub const IC_CONFLICT: char = '\u{f0026}'; // md-alert
pub const IC_WIFIOFF: char = '\u{f05aa}'; // md-wifi_off
pub const IC_ETH: char = '\u{f0200}'; // md-ethernet
pub const IC_ETHOFF: char = '\u{f0202}'; // md-ethernet_cable_off

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

// ---------------------------------------------------------------- verdict

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Offline,
    Portal,
    Open,
    Exposed,
    DnsLeak,
    Conflict,
    Secure,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Offline => "offline",
            Verdict::Portal => "portal",
            Verdict::Open => "open",
            Verdict::Exposed => "exposed",
            Verdict::DnsLeak => "dnsleak",
            Verdict::Conflict => "conflict",
            Verdict::Secure => "secure",
        }
    }

    fn icon(self) -> char {
        match self {
            Verdict::Offline => IC_OFFLINE,
            Verdict::Portal => IC_PORTAL,
            Verdict::Open => IC_OPEN,
            Verdict::Exposed => IC_EXPOSED,
            Verdict::DnsLeak | Verdict::Secure => IC_LOCK,
            Verdict::Conflict => IC_CONFLICT,
        }
    }
}

/// Exact port of net.sh's `classify()` (net.sh:273-306). Pure function, no
/// commands, no globals — `tuns`/`dns` are space-separated tokens, exactly
/// the shape `check()`'s selftest fixtures already pass as shell words.
/// First match wins: offline -> portal -> open/exposed -> dnsleak ->
/// conflict -> secure.
pub fn classify(
    conn: &str,
    sec: &str,
    v4: &str,
    v6: &str,
    tuns: &str,
    dns: &str,
    nconf: u32,
) -> Verdict {
    let is_tun = |d: &str| tuns.split_whitespace().any(|t| t == d);

    if v4.is_empty() && v6.is_empty() {
        return Verdict::Offline;
    }
    match conn {
        "none" => return Verdict::Offline,
        "portal" | "limited" => return Verdict::Portal,
        _ => {}
    }

    // Every family that HAS a default route must ride a tunnel. A
    // v4-tunnelled / v6-direct box is the classic IPv6 leak and lands here
    // too (net.sh:288-297).
    for d in [v4, v6] {
        if !d.is_empty() && !is_tun(d) {
            return match sec {
                "open" | "wep" => Verdict::Open,
                _ => Verdict::Exposed,
            };
        }
    }

    for d in dns.split_whitespace() {
        if d == "local" {
            continue;
        }
        if !is_tun(d) {
            return Verdict::DnsLeak;
        }
    }

    if nconf > 0 {
        return Verdict::Conflict;
    }
    Verdict::Secure
}

// --------------------------------------------------------------- link_sec

fn wifi_seclabel(wifi_row: &str) -> &str {
    wifi_row.split(':').nth(1).unwrap_or("")
}

/// Exact port of net.sh's `link_sec()` (net.sh:247-255), kept pure (no
/// `/sys` read of its own) so it is directly testable and so `regrade()`
/// can reuse the one carrier read it already does for the eth pill instead
/// of reading `/sys/class/net/$ETH_DEV/carrier` twice. `eth_carrier_up`
/// stands in for the shell's `[ "$(cat .../carrier)" = 1 ]`.
///
/// The DELIBERATE fallthrough this ports: an empty/`--` seclabel with a
/// non-empty wifi row is `open` (an open AP, its row still present); an
/// empty seclabel with an EMPTY row means "no wifi row at all" — the shell
/// `case` matches that same `'' | '--'` arm, but the inner `[ -n "$1" ]`
/// test fails, so nothing returns and control falls out of the case
/// statement entirely to the ethernet check below.
pub fn link_sec_pure(wifi_row: &str, eth_carrier_up: bool) -> &'static str {
    match wifi_seclabel(wifi_row) {
        "" | "--" => {
            if !wifi_row.is_empty() {
                return "open";
            }
            // else: fall through to the ethernet carrier check below.
        }
        s if s.contains("WEP") => return "wep",
        _ => return "wpa",
    }
    if eth_carrier_up {
        "wired"
    } else {
        "none"
    }
}

// ---------------------------------------------------------------- wifi_busy

/// Port of net.sh's `wifi_busy()` (net.sh:590-595). The NM device state
/// numeric prefix — before any space — means mid-transition; the
/// parenthetical name is localized, so it is never matched on.
pub fn wifi_busy(nm_state: &str) -> bool {
    let code = nm_state.split_whitespace().next().unwrap_or("");
    matches!(code, "40" | "50" | "60" | "70" | "80" | "90" | "110")
}

// ---------------------------------------------------------------- line_state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineState {
    Busy,
    Idle,
    Ignore,
}

/// Port of net-watch.sh's `line_state()` (net-watch.sh:63-70): classifies
/// one `nmcli monitor` line for `dev`. "connecting" and "connected" are
/// mutually-exclusive literal prefixes (they diverge at the 8th character),
/// so match order does not change the result here — kept in the shell's own
/// busy-before-idle order anyway, to stay an honest port rather than an
/// independent re-derivation.
pub fn line_state(line: &str, dev: &str) -> LineState {
    if line.starts_with(&format!("{dev}: connecting"))
        || line.starts_with(&format!("{dev}: deactivating"))
    {
        return LineState::Busy;
    }
    for suffix in [
        "connected",
        "disconnected",
        "unavailable",
        "unmanaged",
        "unmanageable",
    ] {
        if line.starts_with(&format!("{dev}: {suffix}")) {
            return LineState::Idle;
        }
    }
    LineState::Ignore
}

// -------------------------------------------------------------- rssi/arc

/// Port of net.sh's `rssi_pct()` (net.sh:580): dBm -90..? mapped via
/// `(dbm+90)*100/60`, clamped 0..100, truncated (not rounded).
pub fn rssi_pct(dbm: i32) -> i32 {
    let p = (f64::from(dbm) + 90.0) * 100.0 / 60.0;
    p.clamp(0.0, 100.0) as i32
}

/// Port of net.sh's `rssi_label()` (net.sh:581-585). T19: now called from
/// `refresh_wifi_tip`'s Signal section — was ported ahead of time (T3) so
/// this stage didn't need to re-derive it from the shell a second time.
pub fn rssi_label(dbm: i32) -> &'static str {
    if dbm >= -50 {
        "Excellent"
    } else if dbm >= -60 {
        "Good"
    } else if dbm >= -70 {
        "Fair"
    } else if dbm >= -80 {
        "Weak"
    } else {
        "Very weak"
    }
}

/// Port of net.sh's `arc()` (net.sh:52-58) — four signal-strength icons.
/// NOTE thresholds (>=75/>=50/>=25/else) are DIFFERENT from the CSS pill
/// thresholds (>=60 excellent / >=35 good / else weak, see `wifi_css_class`
/// below) — net.sh keeps the two independent and so does this port.
///
/// T8b: the four Material Symbols "signal_wifi_*" glyphs this used to
/// return all render as wrong CJK substitutes under GTK4 on this system
/// (see IRONBAR.md's T8b entry). No four-level Nerd Font signal-strength
/// set was found at the time; collapsed to one wifi glyph (U+F1EB), with
/// strength still conveyed by `wifi_css_class`'s
/// `.wifi.weak`/`.good`/`.excellent` color classes.
///
/// T19: the real cause of the earlier "not found" was the T17 CSS bug (the
/// icon font stack never reached any label — see spark_module()'s doc
/// comment), not a missing glyph set. With that fixed, `nf-md` does have a
/// four-level `wifi_strength_1..4` ramp — wired in as part of the
/// one-icon-family sweep (IRONBAR.md T19), so signal strength is now
/// conveyed by the glyph itself, not just the CSS color class.
pub fn arc(pct: i32) -> char {
    match pct {
        75..=100 => '\u{f0928}', // md-wifi_strength_4
        50..=74 => '\u{f0925}',  // md-wifi_strength_3
        25..=49 => '\u{f0922}',  // md-wifi_strength_2
        _ => '\u{f091f}',        // md-wifi_strength_1
    }
}

/// Port of net.sh's wifi CSS-class thresholds (net.sh:645-651).
fn wifi_css_class(pct: i32) -> &'static str {
    if pct >= 60 {
        "excellent"
    } else if pct >= 35 {
        "good"
    } else {
        "weak"
    }
}

// -------------------------------------------------------------- tunnels

const TUNNEL_KINDS: &[&str] = &["wireguard", "tun", "ppp", "vti", "vti6", "xfrm"];

/// Kernel-derived tunnel interface set: parses `ip -d -j link show` and
/// returns interface names whose `linkinfo.info_kind` is a recognized
/// tunnel kind AND whose flags include "UP". Port of net.sh's `tunnel_rows`
/// core rule (net.sh:121-151) minus the NM-profile-name join, which only
/// fed the rich tooltip (T7 scope, not the bar text this stage renders).
///
/// NEVER derived from nmcli's DEVICE field — see the module doc and
/// net.sh:123-130 for the security bug this avoids: nmcli reports an
/// OpenVPN connection's DEVICE as the physical link it rides on, so a
/// tunnel set built from nmcli would put the physical interface (e.g.
/// `wlo1`) in the tunnel set and grade a wide-open box "encrypted end to
/// end". `tap` is excluded on purpose (VM bridges, not tunnels).
pub fn tunnel_devices(link_show_json: &Value) -> Vec<String> {
    let Some(arr) = link_show_json.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| {
            let kind = entry.get("linkinfo")?.get("info_kind")?.as_str()?;
            if !TUNNEL_KINDS.contains(&kind) {
                return None;
            }
            let flags = entry.get("flags")?.as_array()?;
            if !flags.iter().any(|f| f.as_str() == Some("UP")) {
                return None;
            }
            entry.get("ifname")?.as_str().map(String::from)
        })
        .collect()
}

// ---------------------------------------------------------------- watch

const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// A restartable line-streaming child (`nmcli monitor` / `ip -o monitor
/// route`) — same restart/backoff/cancel-safety shape as mango.rs's
/// `Watch`, minus the `mmsg get` snapshot pairing: neither command has an
/// analogous "get current state" companion, so there is nothing to pair
/// with. `regrade()` supplies the initial full picture itself, forked once
/// at startup — see `run()` in main.rs.
pub struct MonitorChild {
    cmd: &'static str,
    args: &'static [&'static str],
    lines: Option<Lines<BufReader<ChildStdout>>>,
    child: Option<Child>,
    backoff: Duration,
}

impl MonitorChild {
    pub fn new(cmd: &'static str, args: &'static [&'static str]) -> Self {
        Self {
            cmd,
            args,
            lines: None,
            child: None,
            backoff: BACKOFF_START,
        }
    }

    /// Next line, in the order the child emitted it. Loops internally over
    /// spawn / respawn-after-EOF / backoff — see `Watch::next_doc()`
    /// (mango.rs) for why `Lines::next_line()`'s cancel-safety matters in a
    /// `select!` loop.
    pub async fn next_line(&mut self) -> String {
        loop {
            if self.lines.is_none() {
                if let Err(e) = self.spawn().await {
                    eprintln!("mango-bard: {} {}: {e}", self.cmd, self.args.join(" "));
                    tokio::time::sleep(self.backoff).await;
                    self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                    continue;
                }
                self.backoff = BACKOFF_START;
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
                    self.lines = None;
                    if let Some(mut child) = self.child.take() {
                        let _ = child.start_kill();
                    }
                }
            }
        }
    }

    async fn spawn(&mut self) -> std::io::Result<()> {
        let mut child = Command::new(self.cmd)
            .args(self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout requested above");
        self.lines = Some(BufReader::new(stdout).lines());
        self.child = Some(child);
        Ok(())
    }

    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        self.lines = None;
    }
}

// ------------------------------------------------------------------ Net

pub struct Net {
    wifi_dev: String,
    eth_dev: String,
    pub nmcli_mon: MonitorChild,
    pub route_mon: MonitorChild,
    last_nmcli_line: Option<String>,
    last_route_line: Option<String>,
    /// Mirrors net-watch.sh's `BUSY`: set by `line_state()` classification
    /// of `nmcli monitor` lines, cleared on the matching idle transition.
    nm_busy: bool,
    /// Parity counter for the battery-mode "every second tick" RSSI gate
    /// (decision 2) — full mode ignores it, eco mode never calls in here.
    rssi_tick: u32,
    /// T19: throughput rate state, one entry per iface a detail popup has
    /// asked about — replaces net.sh's `$XDG_RUNTIME_DIR/waybar-net-<iface>`
    /// file (`throughput()`, net.sh:73-96): the daemon already holds state
    /// in-process, so no file is needed. See `throughput_str`/
    /// `throughput_delta`.
    throughput: HashMap<String, ThroughputSample>,
}

/// One iface's previous byte counters + when they were read, for
/// [`Net::throughput_str`]'s rate calculation.
struct ThroughputSample {
    at: Instant,
    rx: i64,
    tx: i64,
}

impl Net {
    pub fn new() -> Self {
        let wifi_dev = std::env::var("MANGO_WIFI_DEV").unwrap_or_else(|_| "wlo1".to_string());
        let eth_dev = std::env::var("MANGO_ETH_DEV").unwrap_or_else(|_| "eno2".to_string());
        Self {
            wifi_dev,
            eth_dev,
            nmcli_mon: MonitorChild::new("nmcli", &["monitor"]),
            route_mon: MonitorChild::new("ip", &["-o", "monitor", "route"]),
            last_nmcli_line: None,
            last_route_line: None,
            nm_busy: false,
            rssi_tick: 0,
            throughput: HashMap::new(),
        }
    }

    /// Raw-line dedup (layer 1, same as mango.rs's `ingest_monitors`) plus
    /// `line_state()` bookkeeping for `nm_busy`. Returns whether this line
    /// is regrade-worthy — true for anything new, even an `Ignore`-
    /// classified line (net-watch.sh sets `PEND=1` unconditionally on every
    /// read event: a "NetworkManager is running" banner still means
    /// `nmcli -f CONNECTIVITY general` may have changed).
    pub fn ingest_nmcli_line(&mut self, line: &str) -> bool {
        if self.last_nmcli_line.as_deref() == Some(line) {
            return false;
        }
        self.last_nmcli_line = Some(line.to_string());
        match line_state(line, &self.wifi_dev) {
            LineState::Busy => self.nm_busy = true,
            LineState::Idle => self.nm_busy = false,
            LineState::Ignore => {}
        }
        true
    }

    pub fn ingest_route_line(&mut self, line: &str) -> bool {
        if self.last_route_line.as_deref() == Some(line) {
            return false;
        }
        self.last_route_line = Some(line.to_string());
        true
    }

    /// Cheap, fork-free recompute of `net_busy`/`wifi_show` — called
    /// immediately on every NM line (so the spinner responds instantly,
    /// matching net-watch.sh's own instant `OUT` flip) independent of the
    /// debounced `regrade()`.
    pub fn refresh_busy_var(&self, vars: &mut Vars) {
        let busy = self.nm_busy || scan_flag_active();
        vars.set("net_busy", if busy { "true" } else { "false" });
        // wifi_show is the logical inverse of net_busy — the spinner
        // REPLACES the wifi pill rather than sitting beside it, preserving
        // net.sh:598-608's "emit busy \"\" \"\"" contract via show_if
        // instead of an empty label.
        vars.set("wifi_show", if busy { "false" } else { "true" });
    }

    /// Pure recompute of the netsec `eco` class from the current powermode
    /// — no forks. Used both inside `regrade()` and standalone by main.rs's
    /// powermode-change arm (which must not fork just to redraw a class).
    pub fn set_eco_class(&self, vars: &mut Vars, pm_mode: powermode::Mode) {
        let eco = if pm_mode != powermode::Mode::Full {
            "eco"
        } else {
            ""
        };
        vars.set(&class_key("netsec#eco"), eco);
    }

    /// Decision 2's RSSI gate: full mode refreshes every tick, battery mode
    /// every second tick, eco mode never (NM events only). Call once per
    /// clock tick; `true` means "call `regrade()` now".
    pub fn should_refresh_on_tick(&mut self, pm_mode: powermode::Mode) -> bool {
        match pm_mode {
            powermode::Mode::Full => true,
            powermode::Mode::Battery => {
                self.rssi_tick = self.rssi_tick.wrapping_add(1);
                self.rssi_tick.is_multiple_of(2)
            }
            powermode::Mode::Eco => false,
        }
    }

    /// Re-derives every network ironvar/class from a fresh fork-and-parse
    /// pass. The only place in net.rs that forks — see the module doc for
    /// the full command list and IRONBAR.md T3 for why this replaces the
    /// original NM D-Bus sketch (decision 1).
    pub async fn regrade(&mut self, vars: &mut Vars, pm_mode: powermode::Mode) {
        let conn = first_line(&run("nmcli", &["-t", "-f", "CONNECTIVITY", "general"]).await);

        let wifi_rows = run(
            "nmcli",
            &["-t", "-f", "ACTIVE,SECURITY,SSID", "dev", "wifi"],
        )
        .await;
        let wifi_row = wifi_rows
            .lines()
            .find(|l| l.starts_with("yes:"))
            .unwrap_or("")
            .to_string();

        let wifi_state = run(
            "nmcli",
            &["-g", "GENERAL.STATE", "device", "show", &self.wifi_dev],
        )
        .await
        .trim()
        .to_string();

        let link_show = run("ip", &["-d", "-j", "link", "show"]).await;
        let link_json: Value = serde_json::from_str(&link_show).unwrap_or(Value::Null);
        let tuns = tunnel_devices(&link_json);
        let tuns_str = tuns.join(" ");

        let v4 = parse_route_get_dev(&run("ip", &["-j", "route", "get", "1.1.1.1"]).await);
        let v6 =
            parse_route_get_dev(&run("ip", &["-j", "route", "get", "2606:4700:4700::1111"]).await);

        let routes4 = parse_routes(&run("ip", &["-j", "-4", "route", "show"]).await, 4);
        let routes6 = parse_routes(&run("ip", &["-j", "-6", "route", "show"]).await, 6);
        let routes: Vec<crate::routes::Route> = routes4.into_iter().chain(routes6).collect();
        let nconf = crate::routes::conflict_scan(&routes).len() as u32;

        let eth_ip4 =
            parse_addr_ip4(&run("ip", &["-j", "-4", "addr", "show", "dev", &self.eth_dev]).await);

        let resolv = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
        let nameservers = parse_nameservers(&resolv);
        let mut dns_devs: Vec<String> = Vec::with_capacity(nameservers.len());
        for ns in &nameservers {
            dns_devs.push(resolver_dev(ns).await);
        }
        let dns_str = dns_devs.join(" ");

        let carrier = std::fs::read_to_string(format!("/sys/class/net/{}/carrier", self.eth_dev))
            .unwrap_or_default();
        let carrier_up = carrier.trim() == "1";

        let sec = link_sec_pure(&wifi_row, carrier_up);
        let verdict = classify(&conn, sec, &v4, &v6, &tuns_str, &dns_str, nconf);

        vars.set(&class_key("netsec"), verdict.as_str());
        self.set_eco_class(vars, pm_mode);
        vars.set("sec_text", barico(verdict.icon()));

        let eth_class = if !carrier_up {
            "disconnected"
        } else if eth_ip4.is_empty() {
            "linked"
        } else {
            "connected"
        };
        let eth_text = match eth_class {
            "disconnected" => barico(IC_ETHOFF),
            "linked" => format!("{} no IP", barico_label(IC_ETH)),
            _ => format!("{} {}", barico_label(IC_ETH), eth_ip4),
        };
        vars.set("eth_text", eth_text);
        vars.set(&class_key("eth"), eth_class);

        self.refresh_busy_var(vars);

        // RSSI: skip the iw fork entirely while mid-transition, same
        // optimization net.sh's wifi_emit() makes (net.sh:609-613).
        if wifi_busy(&wifi_state) {
            return;
        }

        let dump = run("iw", &["dev", &self.wifi_dev, "station", "dump"]).await;
        let link = run("iw", &["dev", &self.wifi_dev, "link"]).await;
        if dump.trim().is_empty() || link.starts_with("Not connected") {
            vars.set("wifi_text", barico(IC_WIFIOFF));
            vars.set(&class_key("wifi"), "disconnected");
            return;
        }

        let rssi = parse_signal_dbm(&dump).unwrap_or(-90);
        let pct = rssi_pct(rssi);
        vars.set("wifi_text", format!("{} {}%", barico_label(arc(pct)), pct));
        vars.set(&class_key("wifi"), wifi_css_class(pct));
    }

    /// T19: rate for `dev` since the last call for that same `dev`, as
    /// "↓ 1.2 MB/s  ↑ 340 kB/s". Port of net.sh's `throughput()`
    /// (net.sh:73-96); the counter-reset/sub-0.2s guard is
    /// [`throughput_delta`], kept pure and tested separately from the
    /// stateful read+store this wraps it in.
    fn throughput_str(&mut self, dev: &str) -> String {
        let Some(c) = read_dev_counters(dev) else {
            return "↓ —  ↑ —".to_string();
        };
        let now = Instant::now();
        let (drx, dtx) = match self.throughput.get(dev) {
            Some(prev) => {
                let elapsed = now.duration_since(prev.at).as_secs_f64();
                throughput_delta(elapsed, c.rx_bytes, prev.rx, c.tx_bytes, prev.tx)
            }
            None => (0, 0),
        };
        self.throughput.insert(
            dev.to_string(),
            ThroughputSample {
                at: now,
                rx: c.rx_bytes,
                tx: c.tx_bytes,
            },
        );
        format!("↓ {}  ↑ {}", human(drx), human(dtx))
    }

    /// Detail popup for the `netsec` pill — full port of net.sh's
    /// `sec_emit()` tooltip body (net.sh:316-494), minus the file-backed tip
    /// cache (that existed only because waybar re-execs a shell every poll;
    /// this daemon holds state and only builds on a hover, see IRONBAR.md
    /// T19). Independent fresh fetch, same shape as `regrade()` — not fed
    /// from `regrade()`'s own last pass, so the popup is never stale by the
    /// time between a real network change and the next scheduled regrade.
    pub async fn refresh_sec_tip(&mut self, vars: &mut Vars) {
        let conn = first_line(&run("nmcli", &["-t", "-f", "CONNECTIVITY", "general"]).await);

        let wifi_rows = run(
            "nmcli",
            &["-t", "-f", "ACTIVE,SECURITY,SSID", "dev", "wifi"],
        )
        .await;
        let wifi_row = wifi_rows
            .lines()
            .find(|l| l.starts_with("yes:"))
            .unwrap_or("")
            .to_string();
        let wifi_ssid = wifi_ssid(&wifi_row);

        let link_show = run("ip", &["-d", "-j", "link", "show"]).await;
        let link_json: Value = serde_json::from_str(&link_show).unwrap_or(Value::Null);
        let tuns = tunnel_devices(&link_json);
        let tuns_str = tuns.join(" ");

        let v4 = parse_route_get_dev(&run("ip", &["-j", "route", "get", "1.1.1.1"]).await);
        let v6 =
            parse_route_get_dev(&run("ip", &["-j", "route", "get", "2606:4700:4700::1111"]).await);

        let routes4 = parse_routes(&run("ip", &["-j", "-4", "route", "show"]).await, 4);
        let routes6 = parse_routes(&run("ip", &["-j", "-6", "route", "show"]).await, 6);
        let routes: Vec<crate::routes::Route> = routes4.into_iter().chain(routes6).collect();
        let conflicts = crate::routes::conflict_scan(&routes);

        let resolv = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
        let nameservers = parse_nameservers(&resolv);
        let mut dns_devs: Vec<String> = Vec::with_capacity(nameservers.len());
        for ns in &nameservers {
            dns_devs.push(resolver_dev(ns).await);
        }
        let dns_str = dns_devs.join(" ");

        let carrier_up =
            std::fs::read_to_string(format!("/sys/class/net/{}/carrier", self.eth_dev))
                .unwrap_or_default()
                .trim()
                == "1";

        let sec = link_sec_pure(&wifi_row, carrier_up);
        let verdict = classify(
            &conn,
            sec,
            &v4,
            &v6,
            &tuns_str,
            &dns_str,
            conflicts.len() as u32,
        );

        let uplink = match sec {
            "open" | "wep" | "wpa" => self.wifi_dev.clone(),
            "wired" => self.eth_dev.clone(),
            _ if !v4.is_empty() => v4.clone(),
            _ => self.wifi_dev.clone(),
        };

        let title_text = verdict_headline(verdict);
        let mut tip = String::new();
        tip.push_str(&sect("\u{f033e}", "Link"));
        match sec {
            "open" | "wep" | "wpa" => {
                tip.push_str(&row(&format!("{} · {}", esc(wifi_ssid), self.wifi_dev)));
                tip.push('\n');
                match sec {
                    "open" => {
                        tip.push_str(&row(&bad(
                            "Open — no encryption, anyone nearby can read this",
                        )));
                    }
                    "wep" => tip.push_str(&row(&bad("WEP — broken, treat it as open"))),
                    _ => tip.push_str(&dim(wifi_seclabel(&wifi_row))),
                }
                tip.push('\n');
                let dump = run("iw", &["dev", &self.wifi_dev, "station", "dump"]).await;
                let mfp = dump_field(&dump, "MFP:", 2).unwrap_or("unknown");
                tip.push_str(&dim(&format!(
                    "Management-frame protection (802.11w): {mfp}"
                )));
                tip.push('\n');
            }
            "wired" => {
                tip.push_str(&row(&format!("Ethernet · {}", self.eth_dev)));
                tip.push('\n');
                tip.push_str(&dim("Wired — the link itself is not encrypted"));
                tip.push('\n');
            }
            _ => {
                tip.push_str(&dim("no uplink"));
                tip.push('\n');
            }
        }

        tip.push_str(&sect("\u{f059f}", "Routes"));
        if !v4.is_empty() {
            let gw = gw_for(&v4, "-4").await;
            let gws = gw.map(|g| format!(" → {g}")).unwrap_or_default();
            if tuns.iter().any(|t| t == &v4) {
                tip.push_str(&row(&format!("IPv4  {}  via {v4}{gws}", good("✓"))));
            } else {
                tip.push_str(&row(&format!(
                    "IPv4  {}  via {v4}{gws} — not tunnelled",
                    bad("✗")
                )));
            }
            tip.push('\n');
        } else {
            tip.push_str(&dim("IPv4  no default route"));
            tip.push('\n');
        }
        if !v6.is_empty() {
            let gw6 = gw_for(&v6, "-6").await;
            let gws = gw6.map(|g| format!(" → {g}")).unwrap_or_default();
            if tuns.iter().any(|t| t == &v6) {
                tip.push_str(&row(&format!("IPv6  {}  via {v6}{gws}", good("✓"))));
            } else {
                tip.push_str(&row(&format!(
                    "IPv6  {}  via {v6}{gws} — leaking outside the tunnel",
                    bad("✗")
                )));
            }
            tip.push('\n');
        } else {
            let n6 = parse_addr_count6(
                &run(
                    "ip",
                    &[
                        "-j", "-6", "addr", "show", "dev", &uplink, "scope", "global",
                    ],
                )
                .await,
            );
            if n6 > 0 {
                tip.push_str(&dim(&format!(
                    "IPv6  address on {uplink} but no default route — unused, not leaking"
                )));
            } else {
                tip.push_str(&dim("IPv6  none"));
            }
            tip.push('\n');
        }

        if !conflicts.is_empty() {
            tip.push_str(&sect("\u{f062c}", "Route conflicts"));
            for c in &conflicts {
                match c.kind {
                    crate::routes::ConflictKind::Tie => {
                        tip.push_str(&row(&format!(
                            "{}  two default routes both at metric {}",
                            mono(&bad("✗")),
                            c.metric_a
                        )));
                        tip.push('\n');
                        tip.push_str(&dim(&format!(
                            "{} and {} — which one wins is arbitrary, set distinct metrics",
                            c.dev_a, c.dev_b
                        )));
                        tip.push('\n');
                    }
                    crate::routes::ConflictKind::Identical => {
                        tip.push_str(&row(&format!(
                            "{}  {} is announced by both {} and {}",
                            mono(&bad("✗")),
                            c.dst_a,
                            c.dev_a,
                            c.dev_b
                        )));
                        tip.push('\n');
                        tip.push_str(&dim(&format!(
                            "metric {} beats {}, so {} never sees this traffic",
                            c.metric_a, c.metric_b, c.dev_b
                        )));
                        tip.push('\n');
                    }
                    crate::routes::ConflictKind::Shadow => {
                        tip.push_str(&row(&format!(
                            "{}  {} via {} sits inside {} via {}",
                            mono(&warn("!")),
                            c.dst_a,
                            c.dev_a,
                            c.dst_b,
                            c.dev_b
                        )));
                        tip.push('\n');
                        tip.push_str(&dim(&format!(
                            "the more specific route wins, so that range leaves via {}, not {}",
                            c.dev_a, c.dev_b
                        )));
                        tip.push('\n');
                    }
                }
            }
        }

        tip.push_str(&sect("\u{f099d}", "Tunnels"));
        let trows = tunnel_rows(&link_json).await;
        if trows.is_empty() {
            tip.push_str(&dim("none active"));
            tip.push('\n');
        } else {
            for (dev, kind, nm) in &trows {
                let ty = match kind.as_str() {
                    "wireguard" => "WireGuard",
                    "tun" => "OpenVPN",
                    other => other,
                };
                let carries = if dev == &v4 && tuns.iter().any(|t| t == &v4) {
                    format!(" {}", good("— carries the default route"))
                } else {
                    String::new()
                };
                let label = if nm.is_empty() { "unmanaged" } else { nm };
                tip.push_str(&row(&format!("{} · {dev} · {ty}{carries}", esc(label))));
                tip.push('\n');
            }
            let carry = [&v4, &v6]
                .into_iter()
                .any(|d| !d.is_empty() && tuns.iter().any(|t| t == d));
            if !carry {
                tip.push_str(&row(&warn("split tunnel — carries no default route")));
                tip.push('\n');
            }
        }

        tip.push_str(&sect("\u{f01d6}", "Resolvers"));
        if nameservers.is_empty() {
            tip.push_str(&dim("none configured"));
            tip.push('\n');
        } else {
            for ns in &nameservers {
                let d = resolver_dev(ns).await;
                let nsp = mono(&format!("{ns:<20}"));
                if d == "local" {
                    tip.push_str(&row(&format!("{}  {nsp} local stub", warn("~"))));
                } else if tuns.iter().any(|t| t == &d) {
                    tip.push_str(&row(&format!("{}  {nsp} {d}", good("✓"))));
                } else {
                    tip.push_str(&row(&format!(
                        "{}  {nsp} {d} — plaintext to the local network",
                        bad("✗")
                    )));
                }
                tip.push('\n');
            }
            let sd = parse_searchdomains(&resolv);
            if !sd.is_empty() {
                tip.push_str(&dim(&format!("search {}", esc(&sd))));
                tip.push('\n');
            }
        }

        if verdict == Verdict::Portal {
            tip.push_str(&sect("\u{f02fc}", "Portal"));
            tip.push_str(&dim(&format!(
                "connectivity: {conn} — click to open the login page"
            )));
            tip.push('\n');
        }

        set_titled(vars, "sec_tip", title_text, tip.trim_end_matches('\n').to_string());
    }

    /// Detail popup for the `wifi` pill — full port of net.sh's
    /// `wifi_emit()` tooltip body (net.sh:597-713), minus the tip file
    /// cache (see `refresh_sec_tip`'s own doc comment for why).
    pub async fn refresh_wifi_tip(&mut self, vars: &mut Vars) {
        let wifi_dev = self.wifi_dev.clone();

        let wifi_state = run(
            "nmcli",
            &["-g", "GENERAL.STATE", "device", "show", &wifi_dev],
        )
        .await
        .trim()
        .to_string();
        if scan_flag_active() || wifi_busy(&wifi_state) {
            // net.sh's `emit busy "" ""` — the spinner (`show_if: #net_busy`)
            // replaces this pill while busy, so there is nothing to show.
            set_titled(vars, "wifi_tip", "Wi-Fi", String::new());
            return;
        }

        let dump = run("iw", &["dev", &wifi_dev, "station", "dump"]).await;
        let link = run("iw", &["dev", &wifi_dev, "link"]).await;
        if dump.trim().is_empty() || link.starts_with("Not connected") {
            let radio = run("nmcli", &["radio", "wifi"]).await;
            let (title_text, tip) = if radio.trim() == "disabled" {
                ("Wi-Fi off", dim("radio disabled — click to enable"))
            } else {
                (
                    "Wi-Fi disconnected",
                    dim(&format!("{wifi_dev} — click to pick a network")),
                )
            };
            set_titled(vars, "wifi_tip", title_text, tip);
            return;
        }

        let rssi = parse_signal_dbm(&dump).unwrap_or(-90);
        let avg = dump_field(&dump, "signal avg:", 3).unwrap_or("?");
        let bcn = dump_field(&dump, "beacon signal avg:", 4).unwrap_or("?");
        let mfp = dump_field(&dump, "MFP:", 2);
        let retry = dump_field(&dump, "tx retries:", 3).unwrap_or("0");
        let failed = dump_field(&dump, "tx failed:", 3).unwrap_or("0");
        let bloss = dump_field(&dump, "beacon loss:", 3).unwrap_or("0");
        let rxdrop = dump_field(&dump, "rx drop misc:", 4).unwrap_or("0");
        let uptime: i64 = dump_field(&dump, "connected time:", 3)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let rxrate = dump_after_colon(&dump, "rx bitrate:").unwrap_or("?");
        let txrate = dump_after_colon(&dump, "tx bitrate:").unwrap_or("?");

        let link_info = parse_iw_link(&link);
        let bssid = link_info.bssid.as_deref().unwrap_or("?");
        let freq_raw = link_info.freq.as_deref().unwrap_or("?");
        let freq_mhz: f64 = link_info
            .freq
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let ssid = link_info.ssid.unwrap_or_default();

        let pct = rssi_pct(rssi);
        let colour = wifi_tip_colour(pct);
        let css_class = wifi_css_class(pct);

        let wifi_rows = run(
            "nmcli",
            &["-t", "-f", "ACTIVE,SECURITY,SSID", "dev", "wifi"],
        )
        .await;
        let wifi_row = wifi_rows
            .lines()
            .find(|l| l.starts_with("yes:"))
            .unwrap_or("")
            .to_string();
        let carrier_up =
            std::fs::read_to_string(format!("/sys/class/net/{}/carrier", self.eth_dev))
                .unwrap_or_default()
                .trim()
                == "1";
        let secraw = link_sec_pure(&wifi_row, carrier_up);

        let title_text = esc(if ssid.is_empty() { "Wi-Fi" } else { &ssid });
        let mut tip = String::new();
        tip.push_str(&sect("\u{f0928}", "Signal"));
        tip.push_str(&row(&format!(
            "{:>3}%  {}  {rssi} dBm",
            pct,
            crate::tooltip::bar(i64::from(pct), colour, 20)
        )));
        tip.push('\n');
        tip.push_str(&dim(&format!(
            "{} · {pct}% · avg {avg} dBm · beacon {bcn} dBm",
            rssi_label(rssi)
        )));
        tip.push('\n');

        tip.push_str(&sect("\u{f033e}", "Security"));
        match secraw {
            "open" => tip.push_str(&row(&bad(
                "Open — unencrypted, anyone nearby can read your traffic",
            ))),
            "wep" => tip.push_str(&row(&bad("WEP — broken, treat it as open"))),
            _ => tip.push_str(&row(&good(wifi_seclabel(&wifi_row)))),
        }
        tip.push('\n');
        match mfp {
            Some("yes") => tip.push_str(&dim(&format!(
                "Management-frame protection (802.11w): {}",
                good("yes")
            ))),
            _ => tip.push_str(&dim(&format!(
                "Management-frame protection (802.11w): {} — deauth attacks possible",
                warn(mfp.unwrap_or("no"))
            ))),
        }
        tip.push('\n');

        tip.push_str(&sect("\u{f05a9}", "Radio"));
        let band = wifi_band(freq_mhz);
        let ch = wifi_channel(freq_mhz);
        let width = bitrate_width(rxrate)
            .map(|w| format!(" · {w}"))
            .unwrap_or_default();
        tip.push_str(&row(&format!(
            "{band} · channel {ch} · {freq_raw} MHz{width}"
        )));
        tip.push('\n');
        tip.push_str(&dim(&format!("BSSID {bssid} · {wifi_dev}")));
        tip.push('\n');

        tip.push_str(&sect("\u{f04c5}", "Throughput"));
        tip.push_str(&row(&self.throughput_str(&wifi_dev)));
        tip.push('\n');
        tip.push_str(&dim(&format!("link ↓ {rxrate}")));
        tip.push('\n');
        tip.push_str(&dim(&format!("link ↑ {txrate}")));
        tip.push('\n');

        tip.push_str(&sect("\u{f02fc}", "Link quality"));
        tip.push_str(&dim(&format!("tx retries {retry} · tx failed {failed}")));
        tip.push('\n');
        tip.push_str(&dim(&format!("beacon loss {bloss} · rx drop {rxdrop}")));
        tip.push('\n');
        tip.push_str(&dim(&format!("connected {}", wifi_uptime_label(uptime))));
        tip.push('\n');

        tip.push_str(&sect("\u{f0a5f}", "Addressing"));
        for line in addr_lines(&wifi_dev).await {
            tip.push_str(&line);
            tip.push('\n');
        }

        vars.set(&class_key("wifi"), css_class);
        set_titled(vars, "wifi_tip", &title_text, tip.trim_end_matches('\n').to_string());
    }

    /// Detail popup for the `eth` pill — full port of net.sh's `eth_emit()`
    /// tooltip body (net.sh:717-781), minus the tip file cache (see
    /// `refresh_sec_tip`'s own doc comment for why).
    pub async fn refresh_eth_tip(&mut self, vars: &mut Vars) {
        let eth_dev = self.eth_dev.clone();
        let carrier = std::fs::read_to_string(format!("/sys/class/net/{eth_dev}/carrier"))
            .unwrap_or_else(|_| "0".to_string());
        let carrier = carrier.trim().to_string();
        let operstate = std::fs::read_to_string(format!("/sys/class/net/{eth_dev}/operstate"))
            .unwrap_or_else(|_| "down".to_string());
        let operstate = operstate.trim().to_string();
        let speed: i64 = std::fs::read_to_string(format!("/sys/class/net/{eth_dev}/speed"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(-1);
        let duplex = std::fs::read_to_string(format!("/sys/class/net/{eth_dev}/duplex"))
            .unwrap_or_else(|_| "unknown".to_string());
        let duplex = duplex.trim().to_string();

        let title_text = format!("Ethernet · {eth_dev}");
        let mut tip = String::new();
        tip.push_str(&sect("\u{f0200}", "Link"));
        if carrier != "1" {
            tip.push_str(&row(&bad("down — no carrier")));
            tip.push('\n');
            // "click to enable the adapter" dropped — the HINTS footer
            // ("click: toggle adapter · right-click: edit connections")
            // already says this; keeping it here was a duplicate.
            tip.push_str(&dim(&format!("operstate {operstate}")));
            tip.push('\n');
        } else {
            if speed > 0 {
                tip.push_str(&row(&format!(
                    "{} · {speed} Mbit/s · {duplex} duplex",
                    good("up")
                )));
            } else {
                tip.push_str(&row(&format!("{} · speed unknown", good("up"))));
            }
            tip.push('\n');
            tip.push_str(&dim(&format!("operstate {operstate}")));
            tip.push('\n');
            tip.push_str(&dim("Wired — the link itself is not encrypted"));
            tip.push('\n');
        }

        tip.push_str(&sect("\u{f0a5f}", "Addressing"));
        for line in addr_lines(&eth_dev).await {
            tip.push_str(&line);
            tip.push('\n');
        }

        tip.push_str(&sect("\u{f04c5}", "Throughput"));
        tip.push_str(&row(&self.throughput_str(&eth_dev)));
        tip.push('\n');

        tip.push_str(&sect("\u{f02fc}", "Counters"));
        match read_dev_counters(&eth_dev) {
            Some(c) => {
                tip.push_str(&dim(&format!(
                    "rx errors {} · rx drops {}",
                    c.rx_errs, c.rx_drop
                )));
                tip.push('\n');
                tip.push_str(&dim(&format!(
                    "tx errors {} · tx drops {}",
                    c.tx_errs, c.tx_drop
                )));
                tip.push('\n');
            }
            None => {
                tip.push_str(&dim("unavailable"));
                tip.push('\n');
            }
        }

        set_titled(vars, "eth_tip", &title_text, tip.trim_end_matches('\n').to_string());
    }
}

impl Default for Net {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- helpers

async fn run(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output().await {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn parse_route_get_dev(json_text: &str) -> String {
    serde_json::from_str::<Value>(json_text)
        .ok()
        .and_then(|v| {
            v.as_array()?
                .first()?
                .get("dev")?
                .as_str()
                .map(String::from)
        })
        .unwrap_or_default()
}

fn parse_routes(json_text: &str, family: u8) -> Vec<crate::routes::Route> {
    let Ok(Value::Array(arr)) = serde_json::from_str(json_text) else {
        return Vec::new();
    };
    arr.into_iter()
        .filter_map(|v| {
            let dev = v.get("dev")?.as_str()?.to_string();
            if dev == "lo" {
                return None;
            }
            let dst = v.get("dst")?.as_str()?.to_string();
            let metric = v.get("metric").and_then(Value::as_u64).unwrap_or(0) as u32;
            Some(crate::routes::Route::new(family, dst, dev, metric))
        })
        .collect()
}

fn parse_addr_ip4(json_text: &str) -> String {
    serde_json::from_str::<Value>(json_text)
        .ok()
        .and_then(|v| {
            let first = v.as_array()?.first()?;
            let ai = first.get("addr_info")?.as_array()?;
            ai.first()?.get("local")?.as_str().map(String::from)
        })
        .unwrap_or_default()
}

fn parse_nameservers(resolv: &str) -> Vec<String> {
    resolv
        .lines()
        .filter_map(|l| l.strip_prefix("nameserver "))
        .map(|s| s.trim().to_string())
        .collect()
}

/// Port of net.sh's `resolver_dev()` (net.sh:236-239): the interface a
/// resolver is reached through, or `"local"` for a loopback stub.
async fn resolver_dev(ns: &str) -> String {
    if ns.starts_with("127.") || ns == "::1" {
        return "local".to_string();
    }
    let dev = parse_route_get_dev(&run("ip", &["-j", "route", "get", ns]).await);
    if dev.is_empty() {
        "-".to_string()
    } else {
        dev
    }
}

/// `iw dev <dev> station dump`'s `\tsignal:  -50 [...] dBm` line -> -50.
/// Deliberately matches only the tab-indented `signal:` line, not `signal
/// avg:`/`beacon signal avg:` — same field net.sh's `fld '^\tsignal:' 2`
/// reads (net.sh:629).
fn parse_signal_dbm(dump: &str) -> Option<i32> {
    for line in dump.lines() {
        if line.starts_with('\t') && line.trim_start().starts_with("signal:") {
            return line.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

/// wifi-menu.sh's rescan pidfile (net.sh:22 `SCAN_FLAG`, wifi-menu.sh's
/// `spin_start`/`spin_stop`) — reused as-is rather than inventing new
/// daemon-side scan-state tracking: it is already the tested mechanism that
/// self-recovers if wifi-menu.sh is killed (the pid stops existing).
/// `/proc/<pid>` existence is a safe, unsafe-free stand-in for `kill -0`.
fn scan_flag_active() -> bool {
    let path = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        .join("wifi-scan");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<u32>() else {
        return false;
    };
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

// ---------------------------------------------------------- T19 helpers
//
// Everything below feeds `refresh_sec_tip`/`refresh_wifi_tip`/
// `refresh_eth_tip` — the detail popups net.sh's `sec_emit`/`wifi_emit`/
// `eth_emit` built, ported in full at T19 (IRONBAR.md).

/// Text HEAD net.sh's `sec_emit()` prints per verdict (net.sh:335-343).
fn verdict_headline(v: Verdict) -> &'static str {
    match v {
        Verdict::Offline => "Offline — no route to the internet",
        Verdict::Portal => "Captive portal — click to sign in",
        Verdict::Open => "Unencrypted link — traffic in the clear",
        Verdict::Exposed => "No tunnel — traffic leaves in the clear",
        Verdict::DnsLeak => "Tunnelled, but DNS leaks",
        Verdict::Conflict => "Encrypted, but routes overlap",
        Verdict::Secure => "Encrypted end to end",
    }
}

/// The 3rd colon-separated field of an `ACTIVE:SECURITY:SSID` row from
/// `nmcli -t -f ACTIVE,SECURITY,SSID dev wifi` — same simple split
/// [`wifi_seclabel`] (the 2nd field) already uses; net.sh's own `wifi_ssid`
/// helper has the same "doesn't unescape nmcli's terse-mode `\:`" limit.
fn wifi_ssid(wifi_row: &str) -> &str {
    wifi_row.splitn(3, ':').nth(2).unwrap_or("")
}

/// `bar()`'s colour argument for the wifi Signal section, keyed to the same
/// three buckets as [`wifi_css_class`] (net.sh:645-651) but returning the
/// tooltip meter colour, not a CSS class name.
fn wifi_tip_colour(pct: i32) -> crate::tooltip::Ink {
    if pct >= 60 {
        crate::tooltip::C_GOOD
    } else if pct >= 35 {
        crate::tooltip::C_WARN
    } else {
        crate::tooltip::C_BAD
    }
}

/// net.sh's inline awk uptime formatter in `wifi_emit()` (net.sh:679-683):
/// "2h 14m" when there's a whole hour, else "14m" — no seconds, no zero
/// padding. Deliberately not [`crate::tooltip::hdur`], which formats
/// differently (adds a seconds branch, zero-pads minutes).
fn wifi_uptime_label(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Band name for a wifi frequency in MHz — net.sh:688 awk arithmetic,
/// ported verbatim. `f64`, not `i64`: live-verified `iw dev <dev> link`
/// prints `freq: 2437.0` on this machine's driver — a trailing `.0` awk
/// tolerates natively (it does all arithmetic in floating point) but an
/// `i64::parse` rejects outright. First integer port of this silently fell
/// back to `freq = 0` on every real link, producing "channel -481 · 0 MHz"
/// live — caught by the T19 live verification pass, not by any unit test
/// (the fixtures below all used bare-integer frequencies).
fn wifi_band(freq: f64) -> &'static str {
    if freq >= 5925.0 {
        "6 GHz"
    } else if freq >= 4900.0 {
        "5 GHz"
    } else {
        "2.4 GHz"
    }
}

/// Channel number for a wifi frequency in MHz — net.sh:689-694 awk
/// arithmetic (`int((m - k) / 5)`), ported verbatim including the
/// truncating (not rounding) division. `f64` in, `i64` out — see
/// `wifi_band`'s doc comment for why the input can't be `i64`.
fn wifi_channel(freq: f64) -> i64 {
    if freq >= 5925.0 {
        ((freq - 5950.0) / 5.0) as i64
    } else if freq >= 4900.0 {
        ((freq - 5000.0) / 5.0) as i64
    } else {
        ((freq - 2407.0) / 5.0) as i64
    }
}

/// Extracts a channel-width token like "80MHz" out of an `iw` bitrate
/// string (e.g. "866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2") — port of
/// net.sh:695's `grep -oE '[0-9]+MHz' | head -1`.
fn bitrate_width(bitrate: &str) -> Option<&str> {
    bitrate
        .split_whitespace()
        .find(|t| t.ends_with("MHz") && t[..t.len() - 3].chars().all(|c| c.is_ascii_digit()))
}

/// One field out of an `iw ... station dump` line, 1-based like awk's `$n`
/// — port of net.sh's `fld()` (net.sh:625: `awk -v k="$1" -v n="$2" '$0 ~ k
/// { print $n; exit }'`). Matches the FIRST dump line containing `key` as a
/// substring, same ambiguity the original awk pattern match has (e.g.
/// "signal avg:" also substring-matches inside "beacon signal avg:" — the
/// dump's own line order, signal avg before beacon signal avg, is what
/// keeps this correct, exactly as it does in the shell).
fn dump_field<'a>(dump: &'a str, key: &str, field_1based: usize) -> Option<&'a str> {
    for line in dump.lines() {
        if line.contains(key) {
            return line.split_whitespace().nth(field_1based - 1);
        }
    }
    None
}

/// Everything after `key` on the first dump line containing it, whitespace-
/// trimmed — port of net.sh's `sub(/^[^:]*:[ \t]*/, "")` idiom used for the
/// rx/tx bitrate lines (net.sh:626-627), which keep the rest of the line
/// (e.g. "866.7 MBit/s VHT-MCS 9") rather than one field.
fn dump_after_colon<'a>(dump: &'a str, key: &str) -> Option<&'a str> {
    for line in dump.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(key) {
            return Some(rest.trim_start());
        }
    }
    None
}

/// BSSID/freq/SSID out of `iw dev <dev> link` — port of net.sh:628-630's
/// three awk one-liners. `freq` stays the raw field text (e.g. `"2437.0"`),
/// exactly as net.sh's own `${FREQ:-?} MHz` display line uses it verbatim
/// with no reformatting — [`wifi_band`]/[`wifi_channel`] parse it to `f64`
/// separately for their own arithmetic (see `wifi_band`'s doc comment for
/// why keeping it as `i64` here broke on a real link).
struct LinkInfo {
    bssid: Option<String>,
    freq: Option<String>,
    ssid: Option<String>,
}

fn parse_iw_link(link: &str) -> LinkInfo {
    let mut info = LinkInfo {
        bssid: None,
        freq: None,
        ssid: None,
    };
    for line in link.lines() {
        if let Some(rest) = line.strip_prefix("Connected to ") {
            info.bssid = rest.split_whitespace().next().map(String::from);
        } else if let Some(rest) = line.trim_start().strip_prefix("freq:") {
            info.freq = rest.split_whitespace().next().map(String::from);
        } else if let Some(rest) = line.trim_start().strip_prefix("SSID:") {
            info.ssid = Some(rest.trim_start().to_string());
        }
    }
    info
}

/// `/proc/net/dev` counters for one iface — port of net.sh's `counters()`
/// (net.sh:64-70): `awk`'s 1-based `b[1] b[3] b[4] b[9] b[11] b[12]` after
/// splitting the post-colon half on whitespace becomes 0-based indices
/// 0/2/3/8/10/11 here.
struct DevCounters {
    rx_bytes: i64,
    rx_errs: i64,
    rx_drop: i64,
    tx_bytes: i64,
    tx_errs: i64,
    tx_drop: i64,
}

fn read_dev_counters(dev: &str) -> Option<DevCounters> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    for line in text.lines() {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        if iface.trim() != dev {
            continue;
        }
        let fields: Vec<i64> = rest
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if fields.len() < 12 {
            return None;
        }
        return Some(DevCounters {
            rx_bytes: fields[0],
            rx_errs: fields[2],
            rx_drop: fields[3],
            tx_bytes: fields[8],
            tx_errs: fields[10],
            tx_drop: fields[11],
        });
    }
    None
}

/// Pure rate calculation for [`Net::throughput_str`] — port of net.sh's
/// `throughput()` reset/staleness guard (net.sh:88-92): a sub-0.2s gap or
/// a counter that went backwards (reboot, iface flap) reports `0, 0`
/// instead of a spike. Split out from the stateful read+store so it is
/// unit-testable without a real clock or `/proc/net/dev`.
fn throughput_delta(elapsed_secs: f64, rx: i64, prev_rx: i64, tx: i64, prev_tx: i64) -> (i64, i64) {
    if elapsed_secs < 0.2 || rx < prev_rx || tx < prev_tx {
        (0, 0)
    } else {
        (
            ((rx - prev_rx) as f64 / elapsed_secs) as i64,
            ((tx - prev_tx) as f64 / elapsed_secs) as i64,
        )
    }
}

/// Default-route gateway for `dev` — port of net.sh's `gw_for()` (net.sh:
/// 155): `ip -j <-4|-6> route show default`, the entry whose `dev` matches.
async fn gw_for(dev: &str, family: &str) -> Option<String> {
    let out = run("ip", &["-j", family, "route", "show", "default"]).await;
    let v: Value = serde_json::from_str(&out).ok()?;
    v.as_array()?
        .iter()
        .find(|e| e.get("dev").and_then(Value::as_str) == Some(dev))
        .and_then(|e| e.get("gateway"))
        .and_then(Value::as_str)
        .map(String::from)
}

/// First address + prefix length (`"addr/prefixlen"`) from an `ip -j addr
/// show` array — port of the `addr/prefixlen` half of net.sh's
/// `addr_rows()` (net.sh:558-575).
fn parse_addr_with_prefix(json_text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json_text).ok()?;
    let first = v.as_array()?.first()?;
    let ai = first.get("addr_info")?.as_array()?;
    let entry = ai.first()?;
    let local = entry.get("local")?.as_str()?;
    let plen = entry.get("prefixlen")?.as_i64()?;
    Some(format!("{local}/{plen}"))
}

/// How many v6 addresses `dev` carries at `scope global` — used only to
/// distinguish "an address exists but no default route" from "no v6 at
/// all" in `refresh_sec_tip`'s Routes section (net.sh:611-615).
fn parse_addr_count6(json_text: &str) -> usize {
    let Ok(v) = serde_json::from_str::<Value>(json_text) else {
        return 0;
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("addr_info")?.as_array())
                .map(std::vec::Vec::len)
                .sum()
        })
        .unwrap_or(0)
}

/// `^search` line of `/etc/resolv.conf`, domains space-joined — port of
/// net.sh's `searchdomains()` (net.sh:231): `awk '/^search/ {$1 = "";
/// print substr($0, 2)}'`. No port existed before T19.
fn parse_searchdomains(resolv: &str) -> String {
    resolv
        .lines()
        .find(|l| l.starts_with("search"))
        .map(|l| l.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// Full address block for `dev` — port of net.sh's `addr_rows()`
/// (net.sh:558-575): IPv4 (+ gateway), IPv6 global (+ gateway), MAC/MTU,
/// DNS servers, search domains. Each element is one already-wrapped
/// `row`/`dim` string with no trailing `\n` (the caller adds it, same
/// convention as every other section here).
async fn addr_lines(dev: &str) -> Vec<String> {
    let mut out = Vec::new();

    let v4 = parse_addr_with_prefix(&run("ip", &["-j", "-4", "addr", "show", "dev", dev]).await);
    let gw4 = gw_for(dev, "-4").await;
    match v4 {
        Some(a) => {
            let gws = gw4.map(|g| format!("  → {g}")).unwrap_or_default();
            out.push(row(&format!("IPv4  {a}{gws}")));
        }
        None => out.push(dim("IPv4  none")),
    }

    let v6 = parse_addr_with_prefix(
        &run(
            "ip",
            &["-j", "-6", "addr", "show", "dev", dev, "scope", "global"],
        )
        .await,
    );
    let gw6 = gw_for(dev, "-6").await;
    match v6 {
        Some(a) => {
            let gws = gw6.map(|g| format!("  → {g}")).unwrap_or_default();
            out.push(row(&format!("IPv6  {a}{gws}")));
        }
        None => out.push(dim("IPv6  none")),
    }

    let mac = std::fs::read_to_string(format!("/sys/class/net/{dev}/address")).unwrap_or_default();
    let mac = mac.trim();
    let mtu = std::fs::read_to_string(format!("/sys/class/net/{dev}/mtu")).unwrap_or_default();
    let mtu = mtu.trim();
    out.push(dim(&format!(
        "MAC {} · MTU {}",
        if mac.is_empty() { "?" } else { mac },
        if mtu.is_empty() { "?" } else { mtu }
    )));

    let resolv = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let ns = parse_nameservers(&resolv).join(" ");
    if !ns.is_empty() {
        out.push(dim(&format!("DNS {ns}")));
    }
    let sd = parse_searchdomains(&resolv);
    if !sd.is_empty() {
        out.push(dim(&format!("search {}", esc(&sd))));
    }

    out
}

/// Tunnel rows for the `netsec` popup's Tunnels section — (dev, kind,
/// nm-profile-name). Full port of net.sh's `tunnel_rows()` (net.sh:133-151)
/// including the NM-profile-name join `tunnel_devices()` (T3) deliberately
/// left out — T3's own doc comment says only bar text was in scope there.
/// The join is matched by IPv4 address, not by device: nmcli reports an
/// OpenVPN connection's DEVICE as the physical link it rides on, not the
/// tunnel interface, so joining by device would mislabel every entry
/// (net.sh:123-130's own security-bug note, still true here).
async fn tunnel_rows(link_show_json: &Value) -> Vec<(String, String, String)> {
    let active = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "TYPE,UUID,NAME",
            "connection",
            "show",
            "--active",
        ],
    )
    .await;
    let mut ip_to_name: HashMap<String, String> = HashMap::new();
    for line in active.lines() {
        let mut parts = line.splitn(3, ':');
        let ty = parts.next().unwrap_or("");
        let uuid = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("");
        if !matches!(ty, "wireguard" | "vpn" | "tun") {
            continue;
        }
        let addr_out = run(
            "nmcli",
            &["-g", "IP4.ADDRESS", "connection", "show", "uuid", uuid],
        )
        .await;
        let Some(addr) = addr_out.lines().next().and_then(|l| l.split('/').next()) else {
            continue;
        };
        if !addr.is_empty() {
            ip_to_name.insert(addr.to_string(), name.to_string());
        }
    }

    let Some(arr) = link_show_json.as_array() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in arr {
        let Some(kind) = entry
            .get("linkinfo")
            .and_then(|li| li.get("info_kind"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !TUNNEL_KINDS.contains(&kind) {
            continue;
        }
        let Some(flags) = entry.get("flags").and_then(Value::as_array) else {
            continue;
        };
        if !flags.iter().any(|f| f.as_str() == Some("UP")) {
            continue;
        }
        let Some(dev) = entry.get("ifname").and_then(Value::as_str) else {
            continue;
        };
        let addr = parse_addr_ip4(&run("ip", &["-j", "-4", "addr", "show", "dev", dev]).await);
        let nm_name = ip_to_name.get(&addr).cloned().unwrap_or_default();
        rows.push((dev.to_string(), kind.to_string(), nm_name));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- classify(): 21 cases, copied verbatim from net.sh:800-825
    // (`# expect conn sec v4 v6 tunnels dns-devs [nconf]`).
    #[test]
    fn classify_matches_net_sh_selftest_table() {
        assert_eq!(classify("full", "wpa", "", "", "", "", 0), Verdict::Offline);
        assert_eq!(
            classify("none", "wpa", "wlo1", "", "", "wlo1", 0),
            Verdict::Offline
        );
        assert_eq!(
            classify("portal", "wpa", "wlo1", "", "", "wlo1", 0),
            Verdict::Portal
        );
        assert_eq!(
            classify("limited", "open", "wlo1", "", "", "wlo1", 0),
            Verdict::Portal
        );
        assert_eq!(
            classify("full", "open", "wlo1", "", "", "wlo1", 0),
            Verdict::Open
        );
        assert_eq!(
            classify("full", "wep", "wlo1", "", "", "wlo1", 0),
            Verdict::Open
        );
        // open AP with a tunnel that isn't carrying the default route.
        assert_eq!(
            classify("full", "open", "wlo1", "", "wg0", "wg0 wlo1", 0),
            Verdict::Open
        );
        assert_eq!(
            classify("full", "wpa", "wlo1", "", "", "wlo1", 0),
            Verdict::Exposed
        );
        assert_eq!(
            classify("full", "wired", "eno2", "", "", "eno2", 0),
            Verdict::Exposed
        );
        // split tunnel: tunnel up, default still on the physical link.
        assert_eq!(
            classify("full", "wpa", "wlo1", "", "wg0", "wg0", 0),
            Verdict::Exposed
        );
        // the IPv6 leak: v4 tunnelled, v6 default still on the AP.
        assert_eq!(
            classify("full", "wpa", "wg0", "wlo1", "wg0", "wg0", 0),
            Verdict::Exposed
        );
        // no v6 default route at all is not a leak.
        assert_eq!(
            classify("full", "wpa", "wg0", "", "wg0", "wg0", 0),
            Verdict::Secure
        );
        assert_eq!(
            classify("full", "open", "wg0", "wg0", "wg0", "wg0 wg0", 0),
            Verdict::Secure
        );
        assert_eq!(
            classify("full", "wpa", "wg0", "", "wg0", "local", 0),
            Verdict::Secure
        );
        assert_eq!(
            classify("full", "wpa", "wg0", "wg0", "wg0 wg1", "wg1 wg0", 0),
            Verdict::Secure
        );
        assert_eq!(
            classify("full", "wpa", "wg0", "", "wg0", "wg0 wlo1", 0),
            Verdict::DnsLeak
        );
        assert_eq!(
            classify("full", "wpa", "wg0", "wg0", "wg0", "wlo1", 0),
            Verdict::DnsLeak
        );
        // route conflicts rank below the leaks, and never mask one.
        assert_eq!(
            classify("full", "wpa", "wg0", "", "wg0", "wg0", 2),
            Verdict::Conflict
        );
        assert_eq!(
            classify("full", "wpa", "wg0", "", "wg0", "wlo1", 2),
            Verdict::DnsLeak
        );
        assert_eq!(
            classify("full", "open", "wlo1", "", "", "wlo1", 3),
            Verdict::Open
        );
        assert_eq!(
            classify("full", "wpa", "wg0", "", "wg0", "wg0", 0),
            Verdict::Secure
        );
    }

    // ---- link_sec(): the deliberate POSIX case-fallthrough (net.sh:247-255).
    #[test]
    fn link_sec_empty_seclabel_nonempty_row_is_open() {
        assert_eq!(link_sec_pure("yes:--:MySSID", false), "open");
        assert_eq!(link_sec_pure("yes::MySSID", true), "open");
    }

    #[test]
    fn link_sec_empty_row_falls_through_to_ethernet_carrier() {
        assert_eq!(link_sec_pure("", true), "wired");
        assert_eq!(link_sec_pure("", false), "none");
    }

    #[test]
    fn link_sec_wep_and_wpa() {
        assert_eq!(link_sec_pure("yes:WEP:MySSID", false), "wep");
        assert_eq!(link_sec_pure("yes:WPA2:MySSID", false), "wpa");
    }

    // ---- wifi_busy(): 9 cases, net.sh:837-845 `busycheck` block.
    #[test]
    fn wifi_busy_matches_net_sh_selftest_table() {
        assert!(!wifi_busy("100 (connected)"));
        assert!(!wifi_busy("30 (disconnected)"));
        assert!(!wifi_busy("20 (unavailable)"));
        assert!(!wifi_busy("10 (unmanaged)"));
        assert!(!wifi_busy("120 (failed)"));
        assert!(wifi_busy("40 (connecting (prepare))"));
        assert!(wifi_busy("70 (connecting (getting IP configuration))"));
        assert!(wifi_busy(
            "90 (connecting (starting secondary connections))"
        ));
        assert!(wifi_busy("110 (deactivating)"));
    }

    // ---- line_state(): 14 cases, net-watch.sh:94-107 selftest (dev=wlo1,
    // verbatim `nmcli monitor` lines).
    #[test]
    fn line_state_matches_net_watch_sh_selftest_table() {
        let dev = "wlo1";
        assert_eq!(line_state("wlo1: deactivating", dev), LineState::Busy);
        assert_eq!(
            line_state("wlo1: connecting (prepare)", dev),
            LineState::Busy
        );
        assert_eq!(
            line_state("wlo1: connecting (configuring)", dev),
            LineState::Busy
        );
        assert_eq!(
            line_state("wlo1: connecting (getting IP configuration)", dev),
            LineState::Busy
        );
        assert_eq!(
            line_state("wlo1: connecting (checking IP connectivity)", dev),
            LineState::Busy
        );
        assert_eq!(
            line_state("wlo1: connecting (starting secondary connections)", dev),
            LineState::Busy
        );
        assert_eq!(line_state("wlo1: connected", dev), LineState::Idle);
        assert_eq!(line_state("wlo1: disconnected", dev), LineState::Idle);
        assert_eq!(line_state("wlo1: unavailable", dev), LineState::Idle);
        assert_eq!(
            line_state(
                "wlo1: using connection 'Albania Tirana - GREEN-FLOOR 3 E'",
                dev
            ),
            LineState::Ignore
        );
        assert_eq!(
            line_state("NetworkManager is running", dev),
            LineState::Ignore
        );
        assert_eq!(
            line_state("Connectivity is now 'full'", dev),
            LineState::Ignore
        );
        // another device transitioning must never arm our spinner.
        assert_eq!(
            line_state("eno2: connecting (prepare)", dev),
            LineState::Ignore
        );
        assert_eq!(
            line_state("There's no primary connection", dev),
            LineState::Ignore
        );
    }

    // ---- rssi_pct / rssi_label / arc threshold boundaries.
    #[test]
    fn rssi_pct_clamps_and_truncates() {
        assert_eq!(rssi_pct(-90), 0);
        assert_eq!(rssi_pct(-30), 100);
        assert_eq!(rssi_pct(-60), 50);
        assert_eq!(rssi_pct(-120), 0); // below floor still clamps to 0
        assert_eq!(rssi_pct(0), 100); // above ceiling still clamps to 100
    }

    #[test]
    fn rssi_label_boundaries() {
        assert_eq!(rssi_label(-50), "Excellent");
        assert_eq!(rssi_label(-51), "Good");
        assert_eq!(rssi_label(-60), "Good");
        assert_eq!(rssi_label(-61), "Fair");
        assert_eq!(rssi_label(-70), "Fair");
        assert_eq!(rssi_label(-71), "Weak");
        assert_eq!(rssi_label(-80), "Weak");
        assert_eq!(rssi_label(-81), "Very weak");
    }

    #[test]
    fn arc_thresholds() {
        // T19: each bucket now has its own glyph — walk the full boundary
        // table and confirm both the exact values and that neighbouring
        // buckets differ.
        assert_eq!(arc(100), '\u{f0928}');
        assert_eq!(arc(75), '\u{f0928}');
        assert_eq!(arc(74), '\u{f0925}');
        assert_eq!(arc(50), '\u{f0925}');
        assert_eq!(arc(49), '\u{f0922}');
        assert_eq!(arc(25), '\u{f0922}');
        assert_eq!(arc(24), '\u{f091f}');
        assert_eq!(arc(0), '\u{f091f}');
    }

    // ---- tunnel guard: configured physical devices are never tunnels,
    // regardless of what a (hypothetical, untrusted) nmcli DEVICE field
    // would claim — tunnel_devices() never reads nmcli at all.
    #[test]
    fn physical_devices_are_never_classified_as_tunnels() {
        let link_show: Value = serde_json::from_str(
            r#"[
                {"ifname":"wlo1","flags":["UP","BROADCAST"]},
                {"ifname":"eno2","flags":["UP","BROADCAST"]},
                {"ifname":"wg0","flags":["UP"],"linkinfo":{"info_kind":"wireguard"}},
                {"ifname":"tun0","flags":["UP"],"linkinfo":{"info_kind":"tun"}},
                {"ifname":"virbr0","flags":["UP"],"linkinfo":{"info_kind":"bridge"}},
                {"ifname":"tap0","flags":["UP"],"linkinfo":{"info_kind":"tap"}},
                {"ifname":"wg1","flags":["BROADCAST"],"linkinfo":{"info_kind":"wireguard"}}
            ]"#,
        )
        .unwrap();
        let tuns = tunnel_devices(&link_show);
        assert_eq!(tuns, vec!["wg0".to_string(), "tun0".to_string()]);
        assert!(!tuns.contains(&"wlo1".to_string()));
        assert!(!tuns.contains(&"eno2".to_string()));
        assert!(!tuns.contains(&"virbr0".to_string()));
        assert!(!tuns.contains(&"tap0".to_string()));
        assert!(!tuns.contains(&"wg1".to_string())); // not UP, excluded
    }

    #[test]
    fn parse_signal_dbm_reads_the_tab_indented_signal_line_only() {
        let dump = "Station aa:bb (on wlo1)\n\tinactive time:\t10 ms\n\tsignal:  \t-45 [-45, -47] dBm\n\tsignal avg:\t-46 dBm\n";
        assert_eq!(parse_signal_dbm(dump), Some(-45));
        assert_eq!(parse_signal_dbm(""), None);
    }

    // ---- regrade_is_idempotent, mirroring mango.rs's apply_is_idempotent:
    // recomputing the same pure state twice must not re-dirty Vars.
    #[test]
    fn regrade_is_idempotent() {
        let mut vars = Vars::new();
        let net = Net::new();
        let verdict = classify("full", "wpa", "wg0", "", "wg0", "wg0", 0);
        vars.set(&class_key("netsec"), verdict.as_str());
        net.set_eco_class(&mut vars, powermode::Mode::Full);
        vars.set("sec_text", barico(verdict.icon()));
        assert!(vars.has_dirty());
        while let Some((k, _)) = vars
            .peek_dirty()
            .map(|(k, v)| (k.to_string(), v.to_string()))
        {
            vars.ack(&k);
        }
        // Re-apply the exact same derived state.
        vars.set(&class_key("netsec"), verdict.as_str());
        net.set_eco_class(&mut vars, powermode::Mode::Full);
        vars.set("sec_text", barico(verdict.icon()));
        assert!(
            !vars.has_dirty(),
            "re-applying unchanged state must dirty nothing"
        );
    }

    #[test]
    fn class_key_and_eco_slot_round_trip_independently() {
        let mut vars = Vars::new();
        vars.set(&class_key("netsec"), "open");
        vars.set(&class_key("netsec#eco"), "eco");
        assert!(vars.has_dirty());
        let mut seen = std::collections::HashSet::new();
        while let Some((k, _)) = vars
            .peek_dirty()
            .map(|(k, v)| (k.to_string(), v.to_string()))
        {
            seen.insert(k.clone());
            vars.ack(&k);
        }
        assert!(seen.contains("@class/netsec"));
        assert!(seen.contains("@class/netsec#eco"));
        assert_eq!(vars.live_value("@class/netsec"), Some("open"));
        assert_eq!(vars.live_value("@class/netsec#eco"), Some("eco"));
        // Changing one must not clobber the other.
        vars.set(&class_key("netsec#eco"), "");
        assert_eq!(vars.live_value("@class/netsec"), Some("open"));
    }

    // ---- T19 additions: the net/security/wifi/eth detail-popup helpers.

    #[test]
    fn throughput_delta_reports_zero_on_a_counter_reset_or_iface_flap() {
        // rx went backwards — reboot or the iface flapped, not a spike.
        assert_eq!(throughput_delta(1.0, 100, 500, 200, 100), (0, 0));
        // tx went backwards.
        assert_eq!(throughput_delta(1.0, 500, 100, 100, 500), (0, 0));
    }

    #[test]
    fn throughput_delta_reports_zero_under_the_02s_staleness_guard() {
        assert_eq!(throughput_delta(0.1, 2048, 1024, 2048, 1024), (0, 0));
    }

    #[test]
    fn throughput_delta_computes_a_real_rate() {
        // 1024 bytes over 1s = 1024 B/s each direction.
        assert_eq!(throughput_delta(1.0, 2048, 1024, 3072, 2048), (1024, 1024));
    }

    #[test]
    fn dump_field_matches_the_first_line_containing_the_key() {
        let dump = "Station aa:bb (on wlo1)\n\ttx retries:\t12\n\ttx failed:\t3\n\tMFP:\tyes\n";
        assert_eq!(dump_field(dump, "tx retries:", 3), Some("12"));
        assert_eq!(dump_field(dump, "tx failed:", 3), Some("3"));
        assert_eq!(dump_field(dump, "MFP:", 2), Some("yes"));
        assert_eq!(dump_field(dump, "no such key", 2), None);
    }

    #[test]
    fn dump_after_colon_keeps_the_whole_rest_of_the_line() {
        let dump = "\trx bitrate:\t866.7 MBit/s VHT-MCS 9 80MHz\n";
        assert_eq!(
            dump_after_colon(dump, "rx bitrate:"),
            Some("866.7 MBit/s VHT-MCS 9 80MHz")
        );
        assert_eq!(dump_after_colon(dump, "tx bitrate:"), None);
    }

    #[test]
    fn parse_iw_link_reads_bssid_freq_and_ssid() {
        let link = "Connected to aa:bb:cc:dd:ee:ff (on wlo1)\n\tSSID: My Network\n\tfreq: 5180\n\tRX: 100 bytes\n";
        let info = parse_iw_link(link);
        assert_eq!(info.bssid.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(info.freq.as_deref(), Some("5180"));
        assert_eq!(info.ssid.as_deref(), Some("My Network"));
    }

    #[test]
    fn parse_iw_link_keeps_the_raw_freq_string_including_a_decimal() {
        // T19 live-verify bug: this machine's driver reports "freq: 2437.0"
        // — a trailing ".0" an i64 parse rejects outright but net.sh's own
        // awk display line ("${FREQ:-?} MHz") shows verbatim.
        let link = "Connected to aa:bb (on wlo1)\n\tfreq: 2437.0\n";
        let info = parse_iw_link(link);
        assert_eq!(info.freq.as_deref(), Some("2437.0"));
    }

    #[test]
    fn parse_iw_link_on_not_connected_yields_nothing() {
        let info = parse_iw_link("Not connected.\n");
        assert!(info.bssid.is_none() && info.freq.is_none() && info.ssid.is_none());
    }

    #[test]
    fn wifi_band_and_channel_boundaries() {
        assert_eq!(wifi_band(2412.0), "2.4 GHz");
        assert_eq!(wifi_channel(2412.0), 1);
        assert_eq!(wifi_band(5180.0), "5 GHz");
        assert_eq!(wifi_channel(5180.0), 36);
        assert_eq!(wifi_band(5955.0), "6 GHz");
        assert_eq!(wifi_channel(5955.0), 1);
    }

    #[test]
    fn wifi_channel_handles_a_fractional_freq_the_same_as_an_integer_one() {
        // The live bug: freq 2437.0 must resolve to the same channel a bare
        // 2437 would (channel 6), not the freq=0.0 fallback's -481.
        assert_eq!(wifi_channel(2437.0), 6);
        assert_eq!(wifi_band(2437.0), "2.4 GHz");
    }

    #[test]
    fn bitrate_width_extracts_the_mhz_token() {
        assert_eq!(
            bitrate_width("866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2"),
            Some("80MHz")
        );
        assert_eq!(bitrate_width("54.0 MBit/s"), None);
    }

    #[test]
    fn wifi_ssid_reads_the_third_colon_field() {
        assert_eq!(wifi_ssid("yes:WPA2:MySSID"), "MySSID");
        assert_eq!(wifi_ssid(""), "");
    }

    #[test]
    fn wifi_uptime_label_drops_seconds_and_omits_zero_hours() {
        assert_eq!(wifi_uptime_label(8040), "2h 14m");
        assert_eq!(wifi_uptime_label(90), "1m");
        assert_eq!(wifi_uptime_label(9), "0m");
    }

    #[test]
    fn verdict_headline_covers_every_verdict() {
        for v in [
            Verdict::Offline,
            Verdict::Portal,
            Verdict::Open,
            Verdict::Exposed,
            Verdict::DnsLeak,
            Verdict::Conflict,
            Verdict::Secure,
        ] {
            assert!(!verdict_headline(v).is_empty());
        }
        assert_ne!(
            verdict_headline(Verdict::Secure),
            verdict_headline(Verdict::Open)
        );
    }

    #[test]
    fn parse_searchdomains_reads_the_search_line() {
        let resolv = "nameserver 1.1.1.1\nsearch example.com corp.local\n";
        assert_eq!(parse_searchdomains(resolv), "example.com corp.local");
        assert_eq!(parse_searchdomains("nameserver 1.1.1.1\n"), "");
    }

    #[test]
    fn parse_addr_with_prefix_reads_the_first_address() {
        let json = r#"[{"addr_info":[{"local":"10.0.0.5","prefixlen":24}]}]"#;
        assert_eq!(
            parse_addr_with_prefix(json),
            Some("10.0.0.5/24".to_string())
        );
        assert_eq!(parse_addr_with_prefix("[]"), None);
    }

    #[test]
    fn parse_addr_count6_counts_across_entries() {
        let json = r#"[{"addr_info":[{"local":"fd00::1"},{"local":"fd00::2"}]}]"#;
        assert_eq!(parse_addr_count6(json), 2);
        assert_eq!(parse_addr_count6("[]"), 0);
    }
}
