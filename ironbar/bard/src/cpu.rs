//! CPU bar pill: text + class only. Ports src/waybar/scripts/cpu.sh's
//! `snapshot()`/`deltas()` (cpu.sh:39-54) and the module's own text/class
//! lines (cpu.sh:359-362).
//!
//! cpu.sh is 444 lines, but only ~15 of them build the pill — the rest
//! (`ps`, `pgrep`, `atop -P PRC`, hwmon, `coregrid`, `core_labels`, `stuck`)
//! exist purely for the tooltip, which IRONBAR.md already assigns to T7
//! ("expensive-tier popups"). T6a therefore adds zero new forks: both
//! collectors in this stage are pure `/proc` reads. Per-core percentages are
//! computed and kept (`percore`) but not yet pushed to a var — T7's
//! coregrid/heatbar is the consumer.

use crate::mango::CLASS_PREFIX;
use crate::tooltip::barico;
use crate::vars::Vars;
use std::collections::HashMap;
use std::time::Duration;

/// chip glyph, cpu.sh:28 (`ic_cpu`).
const IC_CPU: char = '\u{e322}';

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

fn is_cpu_row(name: &str) -> bool {
    name.strip_prefix("cpu")
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// Port of cpu.sh:39-41's `snapshot()`: one `(name, total, idle)` triple per
/// `cpu`/`cpuN` line in `/proc/stat`. `total` sums every numeric field after
/// the name; `idle` is `$5+$6` (idle+iowait) on the full line, matching awk's
/// "missing field reads as 0" semantics for kernels with fewer fields.
pub fn snapshot(proc_stat: &str) -> Vec<(String, u64, u64)> {
    let mut out = Vec::new();
    for line in proc_stat.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        if !is_cpu_row(name) {
            continue;
        }
        let nums: Vec<u64> = fields.filter_map(|f| f.parse().ok()).collect();
        let total: u64 = nums.iter().sum();
        let idle = nums.get(3).copied().unwrap_or(0) + nums.get(4).copied().unwrap_or(0);
        out.push((name.to_string(), total, idle));
    }
    out
}

/// Port of cpu.sh:45-54's `deltas()`: busy percentage per key between two
/// snapshots. A key present in only one sample (core hotplug) is skipped
/// rather than reported as 100%; a counter that runs backwards
/// (suspend/resume) clamps to 0 instead of going negative.
pub fn deltas(prev: &[(String, u64, u64)], cur: &[(String, u64, u64)]) -> Vec<(String, i64)> {
    let prev_map: HashMap<&str, (u64, u64)> = prev
        .iter()
        .map(|(k, t, i)| (k.as_str(), (*t, *i)))
        .collect();
    cur.iter()
        .filter_map(|(name, t, idle)| {
            let (pt, pidle) = *prev_map.get(name.as_str())?;
            let dt = *t as f64 - pt as f64;
            let di = *idle as f64 - pidle as f64;
            let p = if dt > 0.0 {
                (dt - di) * 100.0 / dt + 0.5
            } else {
                0.0
            };
            Some((name.clone(), (p as i64).clamp(0, 100)))
        })
        .collect()
}

/// cpu.sh:359-360: warning at >=90%, normal otherwise. Only those two.
pub fn class_for(total: i64) -> &'static str {
    if total >= 90 {
        "warning"
    } else {
        "normal"
    }
}

fn read_proc_stat() -> String {
    std::fs::read_to_string("/proc/stat").unwrap_or_default()
}

fn set_vars(vars: &mut Vars, total: i64) {
    vars.set("cpu_text", format!("{} {total}%", barico(IC_CPU)));
    vars.set(&class_key("cpu"), class_for(total));
}

pub struct Cpu {
    prev: Option<Vec<(String, u64, u64)>>,
    /// Per-core percentages from the last refresh — held for T7's tooltip,
    /// not yet pushed to a var.
    percore: Vec<(String, i64)>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            prev: None,
            percore: Vec::new(),
        }
    }

    /// cpu.sh:344-350: a fresh daemon has no baseline to diff against, so a
    /// blank pill would sit at 0% until the wheel's first tick fires (up to
    /// 5s in full mode, 60s in eco). Mirrors the shell's fix: take a second
    /// sample 200ms after the first and diff those instead of waiting.
    pub async fn prime(&mut self, vars: &mut Vars) {
        let first = snapshot(&read_proc_stat());
        tokio::time::sleep(Duration::from_millis(200)).await;
        let second = snapshot(&read_proc_stat());
        self.apply(&first, &second, vars);
        self.prev = Some(second);
    }

    /// Wheel/clock-tick refresh: diff against whatever the previous refresh
    /// left behind. No-op (keeps the old snapshot) if there is no baseline
    /// yet — `prime()` is what establishes the first one.
    pub fn refresh(&mut self, vars: &mut Vars) {
        let cur = snapshot(&read_proc_stat());
        if let Some(prev) = self.prev.take() {
            self.apply(&prev, &cur, vars);
        }
        self.prev = Some(cur);
    }

    fn apply(&mut self, prev: &[(String, u64, u64)], cur: &[(String, u64, u64)], vars: &mut Vars) {
        let usage = deltas(prev, cur);
        let total = usage
            .iter()
            .find(|(k, _)| k == "cpu")
            .map(|(_, p)| *p)
            .unwrap_or(0);
        self.percore = usage.into_iter().filter(|(k, _)| k != "cpu").collect();
        set_vars(vars, total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- snapshot()/deltas(): fixtures copied verbatim from cpu.sh:241-254.

    #[test]
    fn snapshot_matches_cpu_sh_selftest_table() {
        let stat = "cpu  100 0 100 700 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\n";
        assert_eq!(
            snapshot(stat),
            vec![
                ("cpu".to_string(), 900, 700),
                ("cpu0".to_string(), 500, 400),
            ]
        );
    }

    #[test]
    fn deltas_matches_cpu_sh_selftest_table() {
        let prev = vec![
            ("cpu".to_string(), 900, 700),
            ("cpu0".to_string(), 500, 400),
        ];
        let cur = vec![
            ("cpu".to_string(), 1000, 725),
            ("cpu0".to_string(), 600, 700),
            ("cpu1".to_string(), 10, 5),
        ];
        // 100 ticks elapsed, 25 idle -> 75% busy. cpu0 went backwards on
        // idle -> clamps to 0. cpu1 has no prior sample -> skipped entirely.
        assert_eq!(
            deltas(&prev, &cur),
            vec![("cpu".to_string(), 75), ("cpu0".to_string(), 0)]
        );
    }

    #[test]
    fn deltas_zero_elapsed_never_goes_negative() {
        let prev = vec![("cpu".to_string(), 1000, 700)];
        let cur = vec![("cpu".to_string(), 1000, 700)];
        assert_eq!(deltas(&prev, &cur), vec![("cpu".to_string(), 0)]);
    }

    #[test]
    fn class_for_matches_cpu_sh_threshold() {
        assert_eq!(class_for(89), "normal");
        assert_eq!(class_for(90), "warning");
        assert_eq!(class_for(100), "warning");
    }

    // ---- idempotency, mirroring net.rs's regrade_is_idempotent: recomputing
    // the same pure state twice must not re-dirty Vars.
    #[test]
    fn set_vars_is_idempotent() {
        let mut vars = Vars::new();
        set_vars(&mut vars, 47);
        assert!(vars.has_dirty());
        while let Some((k, _)) = vars
            .peek_dirty()
            .map(|(k, v)| (k.to_string(), v.to_string()))
        {
            vars.ack(&k);
        }
        set_vars(&mut vars, 47);
        assert!(
            !vars.has_dirty(),
            "re-applying unchanged state must dirty nothing"
        );
    }
}
