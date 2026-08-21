//! Memory bar pill: text + class only. Ports src/waybar/scripts/memory.sh's
//! `mem_kv()` (memory.sh:33) and the module's percent/class/text lines
//! (memory.sh:122-133).
//!
//! Same split as cpu.rs: the pill is `/proc/meminfo` arithmetic; `ps
//! --sort=-rss`, the cached `dmidecode` DIMM table and the page-fault rates
//! are all tooltip, deferred to T7. Unlike cpu, memory needs no prior
//! sample — `refresh()` is correct on the very first call, so there is no
//! `prime()`.

use crate::mango::CLASS_PREFIX;
use crate::tooltip::barico;
use crate::vars::Vars;
use std::collections::HashMap;

/// memory_alt glyph, memory.sh:22 (`ic_mem`).
const IC_MEM: char = '\u{f7a3}';

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

/// Port of memory.sh:33's `mem_kv()`: every `/proc/meminfo` key as
/// `key -> value` (KiB), key stripped of `:`/`(`/`)` exactly as the awk
/// `gsub(/[:()]/, "", $1)` does.
pub fn mem_kv(proc_meminfo: &str) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    for line in proc_meminfo.lines() {
        let mut fields = line.split_whitespace();
        let Some(raw_key) = fields.next() else {
            continue;
        };
        let key: String = raw_key
            .chars()
            .filter(|c| !matches!(c, ':' | '(' | ')'))
            .collect();
        let Some(value) = fields.next().and_then(|v| v.parse::<i64>().ok()) else {
            continue;
        };
        out.insert(key, value);
    }
    out
}

/// Port of memory.sh:123/127's `used*100/total + 0.5` rounding, shared by
/// both the RAM and swap percentages. `total <= 0` reads as 0%, matching the
/// shell's `[ "$SwapTotal" -gt 0 ] && SWAPPCT=...` guard (swap left at the
/// `SWAPPCT=0` default otherwise).
pub fn pct_used(used: i64, total: i64) -> i64 {
    if total <= 0 {
        return 0;
    }
    (used as f64 * 100.0 / total as f64 + 0.5) as i64
}

/// memory.sh:129-131, in order — **last match wins**: `normal`, then
/// `swapping` if any swap is in use, then `warning` if RAM is critical. So
/// `warning` outranks `swapping`, and exactly one class is ever emitted.
pub fn class_for(pct: i64, swap_pct: i64) -> &'static str {
    let mut class = "normal";
    if swap_pct > 0 {
        class = "swapping";
    }
    if pct >= 95 {
        class = "warning";
    }
    class
}

fn set_vars(vars: &mut Vars, pct: i64, swap_pct: i64) {
    vars.set("mem_text", format!("{} {pct}%", barico(IC_MEM)));
    vars.set(&class_key("memory"), class_for(pct, swap_pct));
}

#[derive(Default)]
pub struct Memory;

impl Memory {
    pub fn new() -> Self {
        Self
    }

    pub fn refresh(&mut self, vars: &mut Vars) {
        let kv = mem_kv(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default());
        let total = kv.get("MemTotal").copied().unwrap_or(0);
        let avail = kv.get("MemAvailable").copied().unwrap_or(0);
        let pct = pct_used(total - avail, total);

        let swap_total = kv.get("SwapTotal").copied().unwrap_or(0);
        let swap_free = kv.get("SwapFree").copied().unwrap_or(0);
        let swap_pct = if swap_total > 0 {
            pct_used(swap_total - swap_free, swap_total)
        } else {
            0
        };

        set_vars(vars, pct, swap_pct);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- mem_kv()/pct_used(): fixture copied verbatim from memory.sh:72-86.

    #[test]
    fn mem_kv_matches_memory_sh_selftest_fixture() {
        let meminfo = "MemTotal:       48932736 kB\n\
                        MemFree:         2000000 kB\n\
                        MemAvailable:   30000000 kB\n\
                        Buffers:          500000 kB\n\
                        Cached:         20000000 kB\n\
                        Shmem:           1000000 kB\n\
                        Dirty:              4096 kB\n\
                        Writeback:             0 kB\n\
                        SwapTotal:       4194300 kB\n\
                        SwapFree:        4194300 kB\n";
        let kv = mem_kv(meminfo);
        assert_eq!(kv.get("MemTotal"), Some(&48_932_736));
        assert_eq!(kv.get("MemAvailable"), Some(&30_000_000));
    }

    #[test]
    fn pct_used_zero_total_clamps_to_zero() {
        assert_eq!(pct_used(500, 0), 0);
    }

    #[test]
    fn class_for_order_matches_memory_sh_last_wins() {
        assert_eq!(class_for(10, 0), "normal");
        assert_eq!(class_for(10, 5), "swapping");
        // pct >= 95 overrides swapping even when swap is also active — the
        // shell's ordering (memory.sh:129-131) is load-bearing here.
        assert_eq!(class_for(95, 5), "warning");
        assert_eq!(class_for(95, 0), "warning");
    }

    // ---- idempotency, mirroring net.rs's regrade_is_idempotent.
    #[test]
    fn set_vars_is_idempotent() {
        let mut vars = Vars::new();
        set_vars(&mut vars, 39, 0);
        assert!(vars.has_dirty());
        while let Some((k, _)) = vars
            .peek_dirty()
            .map(|(k, v)| (k.to_string(), v.to_string()))
        {
            vars.ack(&k);
        }
        set_vars(&mut vars, 39, 0);
        assert!(
            !vars.has_dirty(),
            "re-applying unchanged state must dirty nothing"
        );
    }
}
