//! Pairwise route-conflict scanner. Port of net.sh's `conflict_scan()`
//! (net.sh:174-221, the awk pairwise prefix-overlap detector) — see that
//! function's header comment for the full rationale (a more-specific or
//! equal-metric overlapping route silently wins, so traffic believed to be
//! going down one tunnel goes down another).
//!
//! First principles for porting the awk bit-string arithmetic to Rust: awk
//! builds a 32/128-character '0'/'1' string per address and does a prefix
//! `substr` compare because awk has no integer type wide enough for a v6
//! address and no bitwise shift on strings. Rust's `std::net` parses both
//! families into fixed-width integers (`u32`/`u128`) directly, and prefix
//! comparison is `>> (width - m)` equality — same result, no string
//! plumbing needed. This is the "stdlib does it" rung, not a re-derivation
//! of the algorithm: every comparison below matches the awk line by line.

use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Clone, Debug)]
pub struct Route {
    /// 4 or 6.
    pub family: u8,
    /// `"default"`, or an address optionally followed by `/<prefixlen>`
    /// (missing prefix length implies /32 for v4, /128 for v6 — a host
    /// route, exactly like net.sh's awk `len = (fam == 4 ? 32 : 128)`).
    pub dst: String,
    pub dev: String,
    pub metric: u32,
}

impl Route {
    pub fn new(family: u8, dst: impl Into<String>, dev: impl Into<String>, metric: u32) -> Self {
        Self {
            family,
            dst: dst.into(),
            dev: dev.into(),
            metric,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictKind {
    /// Same prefix, two devices — the lower metric wins.
    Identical,
    /// Different prefix lengths overlap — the more specific route wins.
    Shadow,
    /// Two default routes at the same metric — the winner is arbitrary.
    Tie,
}

// `regrade()` (net.rs) uses `conflict_scan(&routes).len()` for classify()'s
// `nconf` count. T19: the per-conflict fields below are now also consumed
// by net.rs's `refresh_sec_tip` (net.sh's `sec_emit` "Route conflicts"
// section, which prints exactly this dst/dev/metric/kind tuple) — the
// `#[allow(dead_code)]` that used to sit here is gone.
#[derive(Clone, Debug)]
pub struct Conflict {
    /// The more-specific (or, for a tie, first-seen) route of the pair.
    pub dst_a: String,
    pub dev_a: String,
    pub metric_a: u32,
    pub dst_b: String,
    pub dev_b: String,
    pub metric_b: u32,
    pub kind: ConflictKind,
}

enum Prefix {
    V4(u32, u8),
    V6(u128, u8),
}

fn parse_prefix(family: u8, dst: &str) -> Option<Prefix> {
    let (addr, len) = match dst.split_once('/') {
        Some((a, l)) => (a, l.parse::<u8>().ok()?),
        None => (dst, if family == 4 { 32 } else { 128 }),
    };
    match family {
        4 => Some(Prefix::V4(u32::from(addr.parse::<Ipv4Addr>().ok()?), len)),
        _ => Some(Prefix::V6(u128::from(addr.parse::<Ipv6Addr>().ok()?), len)),
    }
}

fn top_bits_match_v4(a: u32, b: u32, m: u8) -> bool {
    if m == 0 {
        return true;
    }
    let shift = 32 - u32::from(m);
    (a >> shift) == (b >> shift)
}

fn top_bits_match_v6(a: u128, b: u128, m: u8) -> bool {
    if m == 0 {
        return true;
    }
    let shift = 128 - u32::from(m);
    (a >> shift) == (b >> shift)
}

/// Pairwise prefix-overlap scan. `routes` should exclude `lo` (net.sh's
/// `route_conflicts()` filters that at the `jq` stage — regrade() does the
/// equivalent when it builds the `Route` list, see net.rs).
///
/// ponytail: O(n^2) over the routing table, same complexity the awk already
/// had — a couple hundred comparisons on a normal host. Upgrade path if a
/// full table ever lands here: sort by prefix and compare neighbours.
pub fn conflict_scan(routes: &[Route]) -> Vec<Conflict> {
    let mut defaults: Vec<&Route> = Vec::new();
    let mut regular: Vec<(&Route, Prefix)> = Vec::new();

    for r in routes {
        if r.dst == "default" {
            defaults.push(r);
            continue;
        }
        // IPv6 link-local/multicast: never real conflicts, skip entirely
        // (net.sh: `fam == 6 && (dst ~ /^fe80/ || dst ~ /^ff/)`).
        if r.family == 6 && (r.dst.starts_with("fe80") || r.dst.starts_with("ff")) {
            continue;
        }
        if let Some(p) = parse_prefix(r.family, &r.dst) {
            regular.push((r, p));
        }
    }

    let mut out = Vec::new();
    for i in 0..regular.len() {
        for j in (i + 1)..regular.len() {
            let (ri, pi) = &regular[i];
            let (rj, pj) = &regular[j];
            if ri.family != rj.family || ri.dev == rj.dev {
                continue;
            }
            let (len_i, len_j, overlap) = match (pi, pj) {
                (Prefix::V4(ai, li), Prefix::V4(aj, lj)) => {
                    let m = (*li).min(*lj);
                    (*li, *lj, top_bits_match_v4(*ai, *aj, m))
                }
                (Prefix::V6(ai, li), Prefix::V6(aj, lj)) => {
                    let m = (*li).min(*lj);
                    (*li, *lj, top_bits_match_v6(*ai, *aj, m))
                }
                _ => continue, // families already filtered equal above
            };
            if !overlap {
                continue;
            }
            let kind = if len_i == len_j {
                ConflictKind::Identical
            } else {
                ConflictKind::Shadow
            };
            // The more specific route wins, so it is reported first.
            let (a, b) = if len_i >= len_j {
                (*ri, *rj)
            } else {
                (*rj, *ri)
            };
            out.push(Conflict {
                dst_a: a.dst.clone(),
                dev_a: a.dev.clone(),
                metric_a: a.metric,
                dst_b: b.dst.clone(),
                dev_b: b.dev.clone(),
                metric_b: b.metric,
                kind,
            });
        }
    }

    for i in 0..defaults.len() {
        for j in (i + 1)..defaults.len() {
            let a = defaults[i];
            let b = defaults[j];
            if a.family == b.family && a.metric == b.metric {
                out.push(Conflict {
                    dst_a: "default".to_string(),
                    dev_a: a.dev.clone(),
                    metric_a: a.metric,
                    dst_b: "default".to_string(),
                    dev_b: b.dev.clone(),
                    metric_b: b.metric,
                    kind: ConflictKind::Tie,
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures copied verbatim from net.sh's `scan` block, net.sh:862-892
    // (`R4`/`R6` there build a tab-separated `conflict_scan()` input line;
    // here they build a `Route` directly).
    fn r4(dst: &str, dev: &str, metric: u32) -> Route {
        Route::new(4, dst, dev, metric)
    }
    fn r6(dst: &str, dev: &str, metric: u32) -> Route {
        Route::new(6, dst, dev, metric)
    }

    #[test]
    fn shadow_one_vpn_subnet_swallowed_by_another() {
        // net.sh:866-867
        let out = conflict_scan(&[
            r4("10.10.0.0/20", "tun0", 1000),
            r4("10.10.0.0/16", "wg0", 50),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, ConflictKind::Shadow);
        // The more specific route (/20) is reported first, per net.sh's
        // awk comment "the more specific route wins, so print it first".
        assert_eq!(out[0].dst_a, "10.10.0.0/20");
        assert_eq!(out[0].dev_a, "tun0");
        assert_eq!(out[0].metric_a, 1000);
        assert_eq!(out[0].dst_b, "10.10.0.0/16");
        assert_eq!(out[0].dev_b, "wg0");
        assert_eq!(out[0].metric_b, 50);
    }

    #[test]
    fn identical_same_prefix_two_devices() {
        // net.sh:868-869
        let out = conflict_scan(&[
            r4("10.10.0.0/20", "tun0", 1000),
            r4("10.10.0.0/20", "wg0", 50),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, ConflictKind::Identical);
        assert_eq!(
            (out[0].dev_a.as_str(), out[0].dev_b.as_str()),
            ("tun0", "wg0")
        );
    }

    #[test]
    fn same_device_subnetting_is_not_a_conflict() {
        // net.sh:871-872
        let out = conflict_scan(&[
            r4("10.10.0.0/20", "tun0", 1000),
            r4("10.10.0.0/16", "tun0", 1000),
        ]);
        assert!(out.is_empty());
    }

    #[test]
    fn no_overlap_is_not_a_conflict() {
        // net.sh:873-874
        let out = conflict_scan(&[r4("10.10.0.0/17", "tun0", 0), r4("10.11.0.0/17", "wg0", 0)]);
        assert!(out.is_empty());
    }

    #[test]
    fn host_route_inside_a_subnet_is_shadow() {
        // net.sh:876-877 — missing prefix length implies /32.
        let out = conflict_scan(&[r4("10.10.0.5", "wg0", 50), r4("10.10.0.0/24", "tun0", 1000)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, ConflictKind::Shadow);
    }

    #[test]
    fn multiple_defaults_at_different_metrics_is_normal() {
        // net.sh:879-880 — this is how a VPN takes over.
        let out = conflict_scan(&[r4("default", "wlo1", 600), r4("default", "wg0", 50)]);
        assert!(out.is_empty());
    }

    #[test]
    fn multiple_defaults_at_equal_metric_is_a_tie() {
        // net.sh:881-882
        let out = conflict_scan(&[r4("default", "wlo1", 600), r4("default", "wg0", 600)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, ConflictKind::Tie);
        assert_eq!(out[0].dst_a, "default");
        assert_eq!(out[0].dst_b, "default");
        assert_eq!(
            (out[0].dev_a.as_str(), out[0].dev_b.as_str()),
            ("wlo1", "wg0")
        );
        assert_eq!((out[0].metric_a, out[0].metric_b), (600, 600));
    }

    #[test]
    fn ipv6_64_inside_32_is_shadow() {
        // net.sh:884-885
        let out = conflict_scan(&[
            r6("fd10:10:1::/64", "wg0", 50),
            r6("fd10:10::/32", "tun0", 1000),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, ConflictKind::Shadow);
    }

    #[test]
    fn ipv6_no_overlap() {
        // net.sh:886-887
        let out = conflict_scan(&[
            r6("fd10:10:1::/64", "wg0", 0),
            r6("fd10:10:2::/64", "tun0", 0),
        ]);
        assert!(out.is_empty());
    }

    #[test]
    fn ipv6_link_local_is_ignored() {
        // net.sh:888-889
        let out = conflict_scan(&[r6("fe80::/64", "wg0", 0), r6("fe80::/64", "tun0", 0)]);
        assert!(out.is_empty());
    }

    #[test]
    fn mixed_family_never_cross_matches() {
        // net.sh:891-892 — identical leading bits, different family.
        let out = conflict_scan(&[r4("10.10.0.0/16", "tun0", 0), r6("::/0", "wg0", 0)]);
        assert!(out.is_empty());
    }
}
