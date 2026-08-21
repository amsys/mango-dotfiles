//! The network collector: wifi/eth/netsec pills plus the busy spinner.
//! Ports src/waybar/scripts/net.sh (state machine, RSSI, icons) and
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
use crate::vars::Vars;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

// ------------------------------------------------------------------ icons
//
// Material Symbols Rounded / Nerd Font PUA codepoints — invisible in an
// editor, so named here instead, exactly as net.sh:43-51 does with shell
// functions. Codepoints copied verbatim from net.sh's own comments.

pub const IC_OFFLINE: char = '\u{e2c1}'; // cloud_off
pub const IC_PORTAL: char = '\u{ea77}'; // login
pub const IC_OPEN: char = '\u{f03f}'; // no_encryption
pub const IC_EXPOSED: char = '\u{e898}'; // lock_open
pub const IC_LOCK: char = '\u{e899}'; // lock
pub const IC_CONFLICT: char = '\u{f184}'; // alt_route
pub const IC_WIFIOFF: char = '\u{e648}'; // signal_wifi_off
pub const IC_ETH: char = '\u{eb2f}'; // lan
pub const IC_ETHOFF: char = '\u{e16f}'; // cable_off

/// Same wrapper net.sh's `barico()` (tooltip.sh:65) uses on every icon-
/// bearing bar module, so these pills inherit the arc/lock glyph's size and
/// baseline exactly.
fn barico(icon: char) -> String {
    format!("<span size=\"115%\" rise=\"-1200\">{icon}</span>")
}

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

/// Port of net.sh's `rssi_label()` (net.sh:581-585). Not yet called from
/// `regrade()` — net.sh only used it in the rich wifi tooltip body, which
/// is T7 scope (this stage's genconfig modules have no `popup`, see the
/// module doc). Kept and tested now so the port is complete and ready for
/// T7 to wire in, rather than re-deriving it from the shell a second time.
#[allow(dead_code)]
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
pub fn arc(pct: i32) -> char {
    match pct {
        75..=100 => '\u{e63e}',
        50..=74 => '\u{ebe1}',
        25..=49 => '\u{ebd6}',
        _ => '\u{ebe4}',
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
            "linked" => format!("{} no IP", barico(IC_ETH)),
            _ => format!("{} {}", barico(IC_ETH), eth_ip4),
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
        vars.set("wifi_text", format!("{} {}%", barico(arc(pct)), pct));
        vars.set(&class_key("wifi"), wifi_css_class(pct));
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
        assert_eq!(arc(100), '\u{e63e}');
        assert_eq!(arc(75), '\u{e63e}');
        assert_eq!(arc(74), '\u{ebe1}');
        assert_eq!(arc(50), '\u{ebe1}');
        assert_eq!(arc(49), '\u{ebd6}');
        assert_eq!(arc(25), '\u{ebd6}');
        assert_eq!(arc(24), '\u{ebe4}');
        assert_eq!(arc(0), '\u{ebe4}');
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
}
