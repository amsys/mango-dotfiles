//! Hotspot pill: up/down + tooltip. Ports src/waybar/scripts/hotspot.sh
//! `--status` (lines 121-144) only — `--menu`/`--toggle`/`do_up`/`do_down`
//! stay shell (goal 4: click-driven menus cost nothing at idle, only the
//! polling half moves into the daemon). See IRONBAR.md T6b decision D2.
//!
//! No [`crate::net::MonitorChild`]: there is no reliable file to inotify (the
//! real state — whether `p2p0` exists — has no stable inode to watch across
//! up/down cycles) and no idle-quiet daemon event for it either. Two sources
//! instead: `hotspot.sh`'s `toggle()` pokes `mango-bard refresh hotspot` on
//! an actual state change (hotspot.sh — reached only from `--toggle`/
//! `--menu`, never from `--status`; an earlier version of this poke lived in
//! the script's top-level `EXIT` trap instead, which fired on *every*
//! invocation including `--status` — since waybar's own `custom/hotspot`
//! module is itself signal-driven, that created an unbounded self-feeding
//! refresh loop, found live during this stage's own verification pass and
//! fixed by moving the poke into `toggle()`), and
//! [`Hotspot::should_refresh_on_tick`] is a minute-tick backstop for the
//! client count, gated on `up` — the hotspot is normally down, so the gate
//! returns `false` and this costs nothing in the common case, same shape as
//! `power.rs::should_refresh_on_tick` gating on `discharging`.
//!
//! Link existence is a sysfs stat, not a fork (`ip link show` in the
//! original): `is_up()`/`uplink_label()` become `/sys/class/net/<iface>`
//! stats. Only `iw dev <iface> info` (the channel) still forks, and only
//! while the hotspot is up.

use crate::mango::CLASS_PREFIX;
use crate::tooltip::{barico, kv, rule, sect, title};
use crate::vars::Vars;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// wifi_tethering, Material Symbols Rounded — hotspot.sh:142's own comment
/// flags this codepoint as carried over unverified from the classic Material
/// Icons PUA mapping; same byte sequence ported here regardless.
const IC_HOTSPOT: char = '\u{e1d9}';

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

fn iface() -> String {
    std::env::var("MANGO_HS_IFACE").unwrap_or_else(|_| "p2p0".to_string())
}

/// hotspot.sh:77 `is_up()`, as a sysfs stat instead of `ip link show`.
fn is_up() -> bool {
    Path::new("/sys/class/net").join(iface()).exists()
}

/// hotspot.sh:79-85 `uplink_label()`, as a sysfs stat instead of
/// `ip link show nordlynx`.
fn uplink_label() -> &'static str {
    if Path::new("/sys/class/net/nordlynx").exists() {
        "NordVPN"
    } else {
        "direct"
    }
}

fn cache_path() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join(".cache/mango-hotspot.conf")
}

/// hotspot.sh:71's `SSID=...\nPSK=...\n` format.
fn parse_creds(text: &str) -> (String, String) {
    let mut ssid = String::new();
    let mut psk = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("SSID=") {
            ssid = v.to_string();
        } else if let Some(v) = line.strip_prefix("PSK=") {
            psk = v.to_string();
        }
    }
    (ssid, psk)
}

/// hotspot.sh:67-75 `ensure_creds()`, read-only half: by the time `is_up()`
/// is true, `do_up()` (shell, click-only) has already run `ensure_creds()`
/// and the cache file exists — this never needs to create it, only read it.
fn read_creds() -> (String, String) {
    parse_creds(&std::fs::read_to_string(cache_path()).unwrap_or_default())
}

/// hotspot.sh:87-90 `client_count()`.
fn client_count() -> usize {
    std::fs::read_to_string("/run/mango-hotspot/dnsmasq.leases")
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// hotspot.sh:128 — only fork left, and only while the hotspot is up.
async fn channel() -> String {
    let out = Command::new("iw")
        .args(["dev", &iface(), "info"])
        .output()
        .await;
    let Ok(out) = out else {
        return String::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.contains("channel"))
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string()
}

pub struct Hotspot {
    up: bool,
}

impl Hotspot {
    pub fn new() -> Self {
        Self { up: false }
    }

    /// Client-count backstop (see module doc). Gated on `up`, exactly like
    /// `power.rs::should_refresh_on_tick` gates on `discharging` — costs
    /// nothing while the hotspot is down, which is almost always.
    pub fn should_refresh_on_tick(&self) -> bool {
        self.up
    }

    /// hotspot.sh:121-144.
    pub async fn refresh(&mut self, vars: &mut Vars) {
        self.up = is_up();
        vars.set("hotspot_show", if self.up { "true" } else { "false" });
        if !self.up {
            vars.set("hotspot_text", "");
            vars.set("hotspot_tip", "");
            vars.set(&class_key("hotspot"), "");
            return;
        }

        let (ssid, psk) = read_creds();
        let uplink = uplink_label();
        let chan = channel().await;
        let clients = client_count();

        let mut tip = String::new();
        tip.push_str(&title("Hotspot"));
        tip.push_str(&rule(30));
        tip.push_str(&sect("", &format!("{ssid}  \u{b7}  ch {chan}")));
        tip.push_str(&kv("Password", &psk));
        tip.push_str(&kv("Uplink", uplink));
        tip.push_str(&kv("Clients", &format!("{clients} connected")));

        vars.set("hotspot_text", barico(IC_HOTSPOT));
        vars.set("hotspot_tip", tip.trim_end_matches('\n').to_string());
        vars.set(
            &class_key("hotspot"),
            if uplink == "NordVPN" { "vpn" } else { "active" },
        );
    }
}

impl Default for Hotspot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_refresh_on_tick_gated_on_up() {
        let mut h = Hotspot::new();
        assert!(!h.should_refresh_on_tick(), "down by default");
        h.up = true;
        assert!(h.should_refresh_on_tick());
    }

    #[test]
    fn parse_creds_reads_ssid_and_psk_lines() {
        let (ssid, psk) = parse_creds("SSID=mango-hotspot\nPSK=deadbeefcafef00d\n");
        assert_eq!(ssid, "mango-hotspot");
        assert_eq!(psk, "deadbeefcafef00d");
    }

    #[test]
    fn parse_creds_defaults_empty_on_blank_input() {
        assert_eq!(parse_creds(""), (String::new(), String::new()));
    }
}
