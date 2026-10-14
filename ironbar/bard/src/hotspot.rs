//! Hotspot pill: up/down + tooltip. Ports src/ironbar/scripts/hotspot.sh
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
//! original): `is_up()` is a `/sys/class/net/<iface>` stat. While the
//! hotspot is up, three forks run: `iw dev <iface> info` (the channel),
//! `ip -o route get 1.1.1.1` and `nmcli ... connection show --active`
//! (the uplink: a VPN name, or `direct`). The protected flag is the file
//! `/run/mango-hotspot/protected`. A protected+VPN up to VPN down edge sends
//! one desktop notification.

use crate::cmd::run;
use crate::mango::CLASS_PREFIX;
use crate::tooltip::{barico, kv, sect, set_titled};
use crate::vars::Vars;
use std::path::{Path, PathBuf};

/// wifi_tethering, was Material Symbols Rounded U+E1D9 — hotspot.sh:142's
/// own comment flags this codepoint as carried over unverified from the
/// classic Material Icons PUA mapping.
/// T8b: -> U+F1EB (the same wifi-fan glyph net.rs's `arc()` used to return
/// for every bucket) — GTK4 cannot correctly rasterize Material Symbols
/// Rounded's variable font on this system, and the first replacement tried
/// (U+F012, signal bars) also rendered as wrong CJK tofu live in ironbar;
/// U+F1EB was confirmed working by an actual live-ironbar screenshot
/// (already proven for the wifi pill itself). See IRONBAR.md's T8b entry.
/// T19: U+F1EB -> U+F0003 (md-access_point) — one-icon-family sweep
/// (IRONBAR.md T19). `arc()` (net.rs) now returns four distinct signal-
/// strength glyphs instead of one shared constant, so the old "reuse the
/// wifi pill's own confirmed glyph" argument for the shared codepoint no
/// longer holds either — an access-point glyph reads more precisely for
/// "broadcasting", not "receiving", a wifi signal.
const IC_HOTSPOT: char = '\u{f0003}';

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

const PROTECTED_FLAG: &str = "/run/mango-hotspot/protected";

/// The `dev` field of one `ip -o route get` line.
fn route_dev(line: &str) -> Option<String> {
    let mut it = line.split_whitespace();
    while let Some(w) = it.next() {
        if w == "dev" {
            return it.next().map(str::to_string);
        }
    }
    None
}

/// NAME of the active VPN connection (type wireguard, vpn or tun) on `dev`,
/// from `nmcli -t -f TYPE,DEVICE,NAME` lines. NAME comes last and may hold
/// ':', which `nmcli -t` writes as `\:`.
fn vpn_name_for_dev(nmcli: &str, dev: &str) -> Option<String> {
    nmcli.lines().find_map(|l| {
        let mut p = l.splitn(3, ':');
        let (ty, d, name) = (p.next()?, p.next()?, p.next()?);
        (matches!(ty, "wireguard" | "vpn" | "tun") && d == dev && !name.is_empty())
            .then(|| name.replace("\\:", ":"))
    })
}

/// `NordVPN (al134.nordvpn.com)` -> `NordVPN (al134)`.
fn short_label(name: &str) -> String {
    name.replace(".nordvpn.com", "")
}

/// Name of the VPN that carries the default route, if any (SPEC §3).
async fn uplink_vpn() -> Option<String> {
    let route = run("ip", &["-o", "route", "get", "1.1.1.1"]).await;
    let dev = route.lines().find_map(route_dev)?;
    let nm = run(
        "nmcli",
        &["-t", "-f", "TYPE,DEVICE,NAME", "connection", "show", "--active"],
    )
    .await;
    vpn_name_for_dev(&nm, &dev).map(|n| short_label(&n))
}

/// Notify only on the edge protected+VPN up -> protected+VPN down, and only
/// when the previous state is known (`prev` is `None` on the first refresh).
fn vpn_down_edge(prev: Option<(bool, bool, bool)>, up: bool, protected: bool, vpn_up: bool) -> bool {
    matches!(prev, Some((true, true, true))) && up && protected && !vpn_up
}

/// Tooltip text of the Uplink row.
fn uplink_row(vpn: Option<&str>, protected: bool) -> String {
    let base = vpn.unwrap_or("direct");
    let tag = match (protected, vpn.is_some()) {
        (true, true) => "  \u{2713} protected",
        (true, false) => "  \u{26a0} VPN down",
        (false, _) => "  \u{26a0} not protected",
    };
    format!("{base}{tag}")
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
    run("iw", &["dev", &iface(), "info"])
        .await
        .lines()
        .find(|l| l.contains("channel"))
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string()
}

pub struct Hotspot {
    up: bool,
    /// Last (up, protected, vpn_up); `None` before the first refresh.
    last: Option<(bool, bool, bool)>,
}

impl Hotspot {
    pub fn new() -> Self {
        Self { up: false, last: None }
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
        if !self.up {
            self.last = Some((false, false, false));
        }
        vars.set("hotspot_show", if self.up { "true" } else { "false" });
        if !self.up {
            vars.set("hotspot_text", "");
            vars.set("hotspot_tip", "");
            vars.set("hotspot_tip_title", "");
            vars.set(&class_key("hotspot"), "");
            return;
        }

        let (ssid, psk) = read_creds();
        let vpn = uplink_vpn().await;
        let protected = Path::new(PROTECTED_FLAG).exists();
        let vpn_up = vpn.is_some();
        if vpn_down_edge(self.last, self.up, protected, vpn_up) {
            tokio::spawn(async {
                run(
                    "notify-send",
                    &["-a", "mango-bard", "Hotspot", "VPN down \u{2014} hotspot clients have no internet"],
                )
                .await;
            });
        }
        self.last = Some((self.up, protected, vpn_up));
        let chan = channel().await;
        let clients = client_count();

        let mut tip = String::new();
        tip.push_str(&sect("", &format!("{ssid}  \u{b7}  ch {chan}")));
        tip.push_str(&kv("Password", &psk));
        tip.push_str(&kv("Uplink", &uplink_row(vpn.as_deref(), protected)));
        tip.push_str(&kv("Clients", &format!("{clients} connected")));

        vars.set("hotspot_text", barico(IC_HOTSPOT));
        // T15: hotspot's static tooltip is gone (see genconfig.rs's
        // hotspot_module doc comment) — its gesture hint lives in
        // tooltip::HINTS now, so the popup needs the same set_titled() path
        // every other converted module uses to append it.
        set_titled(
            vars,
            "hotspot_tip",
            "Hotspot",
            tip.trim_end_matches('\n').to_string(),
        );
        vars.set(
            &class_key("hotspot"),
            if vpn_up { "vpn" } else { "active" },
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
    fn route_dev_reads_dev_field() {
        assert_eq!(
            route_dev("1.1.1.1 dev wg_nord_al134 table 51820 src 10.5.0.2 uid 1000"),
            Some("wg_nord_al134".to_string())
        );
        assert_eq!(route_dev("garbage"), None);
    }

    #[test]
    fn vpn_name_matches_dev_and_keeps_colons() {
        let nm = "802-11-wireless:wlan0:home\nwireguard:wg_nord_al134:NordVPN (al134.nordvpn.com)\nvpn:tun0:corp\\:a\\:b\n";
        assert_eq!(
            vpn_name_for_dev(nm, "wg_nord_al134").as_deref(),
            Some("NordVPN (al134.nordvpn.com)")
        );
        assert_eq!(vpn_name_for_dev(nm, "tun0").as_deref(), Some("corp:a:b"));
        assert_eq!(vpn_name_for_dev(nm, "wlan0"), None);
        assert_eq!(vpn_name_for_dev("", "wlan0"), None);
    }

    #[test]
    fn short_label_drops_nordvpn_host() {
        assert_eq!(short_label("NordVPN (al134.nordvpn.com)"), "NordVPN (al134)");
        assert_eq!(short_label("corp"), "corp");
    }

    #[test]
    fn uplink_row_variants() {
        assert_eq!(uplink_row(Some("X"), true), "X  \u{2713} protected");
        assert_eq!(uplink_row(None, true), "direct  \u{26a0} VPN down");
        assert_eq!(uplink_row(None, false), "direct  \u{26a0} not protected");
        assert_eq!(uplink_row(Some("X"), false), "X  \u{26a0} not protected");
    }

    #[test]
    fn vpn_down_edge_only_on_protected_loss() {
        let full = Some((true, true, true));
        assert!(vpn_down_edge(full, true, true, false));
        assert!(!vpn_down_edge(None, true, true, false), "first refresh");
        assert!(!vpn_down_edge(Some((true, true, false)), true, true, false), "already down");
        assert!(!vpn_down_edge(full, true, true, true), "still up");
        assert!(!vpn_down_edge(full, true, false, false), "not protected");
        assert!(!vpn_down_edge(Some((true, false, true)), true, true, false));
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
