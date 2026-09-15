//! CPU bar pill (T6a) plus the detail popup (T7a). Ports
//! src/waybar/scripts/cpu.sh in full: `snapshot()`/`deltas()` (cpu.sh:39-54)
//! and the pill's own text/class lines (cpu.sh:359-362) at T6a; `core_labels`,
//! `cpu_freq`, `cpu_policy`, the package-temp hwmon scan and `stuck`
//! (cpu.sh's tooltip half) at T7a. T7a also ported a `coregrid` per-core
//! text grid, dropped at T32 (its own heatbar sparkline row already shows
//! the same per-core data in one line, and the popup shares a screen-height
//! budget with mem_tip).
//!
//! T7a design (IRONBAR.md): the popup is lazy (D1/D2) — `refresh_detail` runs
//! only when the control socket's `cpu-detail` topic fires, which is only
//! ever poked from inside the popup's own `{{interval:cmd}}` script while it
//! is open (main.rs, genconfig.rs). `refresh()` (the T6a pill path, on the
//! wheel) stays pure `/proc` reads with zero forks either way.

use crate::cmd::run;
use crate::mango::CLASS_PREFIX;
use crate::tooltip::{
    bad, bar, barico_label, dim, esc, grade, hdur, heatbar, level_class, mono, row, sect,
    set_titled, C_DIM,
};
use crate::vars::Vars;
use std::collections::HashMap;
use std::time::Duration;

/// chip glyph, cpu.sh:28 (`ic_cpu`).
/// T8b: U+E322 (Material Symbols "memory") -> U+F2DB (chip, JetBrainsMono
/// Nerd Font Font Awesome) — GTK4 cannot correctly rasterize Material
/// Symbols Rounded's variable font on this system; see IRONBAR.md's T8b
/// entry. NOTE: the first replacement tried here, U+F4BC, also failed —
/// verified with pango-view outside GTK4, then rendered wrong (CJK tofu)
/// live in ironbar anyway. pango-view success does not predict a correct
/// GTK4 render for Nerd Font glyphs either; only U+F2DB was confirmed by
/// loading it in a real ironbar instance and screenshotting the result.
/// T19: U+F2DB -> U+F0EE0 (md-cpu_64_bit) — one-icon-family sweep
/// (IRONBAR.md T19); U+F2DB (Font Awesome) sat smaller and higher than its
/// Material Design neighbours on the bar.
/// T23: U+F0EE0 -> U+F04C5 (md-speedometer, `IC_LOAD` below). The old
/// glyph bakes a literal "64" into its ink; at the bar's 15px font-size
/// those digits collapse into two grey smudges inside a toothed ring,
/// reading as a generic settings gear rather than a processor (confirmed
/// by rendering candidates at 15px with the live Propo font, not just
/// eyeballing a 96px preview). The bar pill's own number is the total load
/// percentage, so the gauge glyph already used for the popup's "Load"
/// section is the correct icon for it, not just a legible one — no
/// "one glyph, two things" conflict, since both spots mean the same thing.
const IC_CPU: char = '\u{f04c5}';

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
    // T28: digit dropped entirely — the `.gauge` fill (genconfig.rs/
    // style.css) now carries the magnitude on the bar; the exact number
    // still lives in the popup. This is what the 353px->~214px reduction
    // target actually requires (`docs/statusbar-layout.md` §3.4) — icon
    // alone, not icon+number, is what makes the pill narrower than before
    // despite gaining a gauge. `#level` is an independent class slot
    // (vars.rs/mango.rs's `CLASS_PREFIX` doc comment), so it never evicts
    // `class_for`'s own warning/normal class.
    //
    // T29: both slot keys move under `sysload` — `cpu` shares that node
    // with `memory` now (`sysload_module()`'s own doc comment) — and both
    // values gain a `cpu-`/`cl` prefix. `class_for` itself stays bare
    // ("warning"/"normal", test-locked below): the prefix is applied only
    // here, at the point the value is written to the shared node. Without
    // it, memory's own bare "warning" would collide on that one node —
    // `ipc.rs::set_class` removes an OLD value before adding the NEW one,
    // so cpu clearing its own "warning" would delete memory's "warning" off
    // the same GTK widget too, even though memory never changed.
    vars.set("cpu_text", barico_label(IC_CPU));
    vars.set(
        &class_key("sysload#cpu"),
        format!("cpu-{}", class_for(total)),
    );
    vars.set(&class_key("sysload#cpulevel"), level_class("cl", total));
}

pub struct Cpu {
    prev: Option<Vec<(String, u64, u64)>>,
    /// Per-core percentages from the last refresh, consumed by
    /// `refresh_detail`'s "Cores" section.
    percore: Vec<(String, i64)>,
    /// Aggregate percentage from the last refresh — the same number the
    /// pill shows, needed again for the popup's "Load" row.
    total: i64,
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
            total: 0,
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
        self.total = total;
        set_vars(vars, total);
    }

    /// T7a: builds `cpu_tip` for the detail popup. Called only from the
    /// control socket's `cpu-detail` topic — never from the wheel/clock path
    /// — so nothing here runs while the popup is closed (IRONBAR.md T7
    /// acceptance line). `ps` is the only fork; the wheel-driven pill
    /// (`refresh`, above) still forks nothing.
    pub async fn refresh_detail(&mut self, vars: &mut Vars) {
        let cpu_sys = cpu_sys_root();
        let ncore = self.percore.len();
        let freqs = read_core_freqs(ncore, &cpu_sys);
        // `labels` (core_labels' second field) fed the popup's per-core
        // text grid, dropped at T32 — the heatbar sparkline already shows
        // the same per-core percentages in one line, so `gap` is the only
        // thing still needed out of `core_labels`.
        let (gap, _labels) = core_labels(&freqs);
        let percts: Vec<i64> = self.percore.iter().map(|(_, p)| *p).collect();

        let loadavg = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let freq_line = cpu_freq(&cpuinfo);
        let (gov, epp, no_turbo) = read_cpu_policy_files(&cpu_sys);
        let policy_line = cpu_policy(&gov, &epp, &no_turbo);
        let temp_c = package_temp(&read_hwmon_temps("/sys/class/hwmon"));

        let top_ps = ps_top_cpu().await;
        let stuck_rows = ps_stuck().await;

        let tip = build_tip(&DetailInputs {
            total: self.total,
            ncore,
            percts: &percts,
            gap,
            loadavg: &loadavg,
            freq_line: &freq_line,
            policy_line: &policy_line,
            temp_c,
            top_ps: &top_ps,
            stuck_rows: &stuck_rows,
        });
        set_titled(vars, "cpu_tip", "CPU", tip);
    }
}

// ---------------------------------------------------------------- T7a icons

const IC_LOAD: char = '\u{f04c5}'; // md-speedometer, cpu.sh:33
const IC_CORES: char = '\u{f061a}'; // md-chip, cpu.sh:34
const IC_TOP: char = '\u{f0279}'; // md-format_list_bulleted, cpu.sh:35
const IC_STUCK: char = '\u{f002a}'; // md-alert, cpu.sh:36

fn cpu_sys_root() -> String {
    std::env::var("MANGO_CPU_SYS").unwrap_or_else(|_| "/sys/devices/system/cpu".to_string())
}

// ------------------------------------------------------------- core labels

/// Glue: one `cpuinfo_max_freq` read per core, 0 for a missing/unreadable
/// file (matches cpu.sh:112's `((getline v < p) > 0) ? v + 0 : 0`).
fn read_core_freqs(ncore: usize, cpu_sys: &str) -> Vec<i64> {
    (0..ncore)
        .map(|i| {
            std::fs::read_to_string(format!("{cpu_sys}/cpu{i}/cpufreq/cpuinfo_max_freq"))
                .ok()
                .and_then(|s| s.trim().parse::<i64>().ok())
                .unwrap_or(0)
        })
        .collect()
}

/// Port of cpu.sh:107-127's `core_labels()`: the fastest max-frequency
/// present is `P`, anything slower is `E`; a uniform CPU (or one with no
/// cpufreq at all — every reading 0) falls back to plain `c0..cN`. Returns
/// `(gap, labels)`, where `gap` is the 1-based index of the first E-core (0
/// if there is no split), exactly where [`heatbar`] should break the row.
pub fn core_labels(freqs: &[i64]) -> (usize, Vec<String>) {
    let top = freqs.iter().copied().max().unwrap_or(0);
    let hybrid = freqs.iter().any(|&f| f != top);
    let mut gap = 0usize;
    let labels = freqs
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            if !hybrid || top == 0 {
                format!("c{i}")
            } else {
                let is_p = f == top;
                if !is_p && gap == 0 {
                    gap = i + 1;
                }
                format!("{}{i}", if is_p { "P" } else { "E" })
            }
        })
        .collect();
    (gap, labels)
}

/// Port of cpu.sh:160-171's `cpu_freq()`: average `/proc/cpuinfo` "cpu MHz"
/// lines in GHz, plus the spread across cores when it is at least 50 MHz —
/// otherwise a hybrid chip's average alone would hide that the P-cores are
/// boosting while the E-cores sit at their floor. Empty string with no data.
pub fn cpu_freq(proc_cpuinfo: &str) -> String {
    let mut sum = 0.0f64;
    let mut n = 0i64;
    let mut mn = 0.0f64;
    let mut mx = 0.0f64;
    for line in proc_cpuinfo.lines() {
        if !line.starts_with("cpu MHz") {
            continue;
        }
        let Some(val) = line
            .split_whitespace()
            .nth(3)
            .and_then(|v| v.parse::<f64>().ok())
        else {
            continue;
        };
        sum += val;
        n += 1;
        if mn == 0.0 || val < mn {
            mn = val;
        }
        if val > mx {
            mx = val;
        }
    }
    if n == 0 {
        return String::new();
    }
    let avg = sum / n as f64 / 1000.0;
    let mut out = format!("{avg:.2} GHz avg");
    if mx - mn >= 50.0 {
        out.push_str(&format!("  ·  {:.2}–{:.2} GHz", mn / 1000.0, mx / 1000.0));
    }
    out
}

/// Glue: the three optional single-line sysfs files `cpu_policy` joins.
/// Empty string for anything missing (AMD has no EPP, a kernel without
/// intel_pstate has no `no_turbo` — cpu.sh:176-179).
fn read_cpu_policy_files(cpu_sys: &str) -> (String, String, String) {
    let read = |rel: &str| -> String {
        std::fs::read_to_string(format!("{cpu_sys}/{rel}"))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    (
        read("cpu0/cpufreq/scaling_governor"),
        read("cpu0/cpufreq/energy_performance_preference"),
        read("intel_pstate/no_turbo"),
    )
}

/// Port of cpu.sh:176-189's `cpu_policy()`: governor, EPP and turbo state
/// joined by `"  ·  "`, each present only if its file actually read.
pub fn cpu_policy(gov: &str, epp: &str, no_turbo: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !gov.is_empty() {
        parts.push(gov);
    }
    if !epp.is_empty() {
        parts.push(epp);
    }
    match no_turbo {
        "0" => parts.push("turbo on"),
        "1" => parts.push("turbo off"),
        _ => {}
    }
    parts.join("  ·  ")
}

/// Glue: every `hwmon*/name` + `temp1_input` pair readable under `hwmon_root`
/// (`/sys/class/hwmon` in production; a tempdir in a real filesystem test
/// would work equally well, but none is pinned here — cpu.sh's own inline
/// version has no selftest either, see cpu.sh:388-395).
fn read_hwmon_temps(hwmon_root: &str) -> Vec<(String, i64)> {
    let Ok(entries) = std::fs::read_dir(hwmon_root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = std::fs::read_to_string(path.join("name")).ok()?;
            let milli = std::fs::read_to_string(path.join("temp1_input"))
                .ok()?
                .trim()
                .parse::<i64>()
                .ok()?;
            Some((name.trim().to_string(), milli))
        })
        .collect()
}

/// Port of cpu.sh:390-395's inline package-temp scan: the package sensor is
/// picked by name (`coretemp`/`k10temp`/`zenpower`), never a bare max over
/// every hwmon — that would report whichever nvme happens to run hottest.
pub fn package_temp(sensors: &[(String, i64)]) -> Option<i64> {
    sensors
        .iter()
        .filter(|(name, _)| matches!(name.as_str(), "coretemp" | "k10temp" | "zenpower"))
        .map(|(_, milli)| *milli)
        .max()
}

fn format_temp(milli: i64) -> String {
    format!("{:.0}°C", milli as f64 / 1000.0)
}

// -------------------------------------------------------- ps/top processes

/// Splits leading whitespace-delimited numeric tokens off the front of a
/// `ps` line while leaving the tail (which may itself contain spaces — a
/// browser's helper processes are named things like "Isolated Web Co") for
/// the caller to take verbatim. Mirrors shell `read`'s own behaviour of
/// collapsing separator whitespace but preserving it inside the last field.
fn take_tok(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    s.split_at(end)
}

fn parse_pcpu_line(line: &str) -> Option<(String, i64, String)> {
    let (pcpu, rest) = take_tok(line);
    let (pid_s, rest) = take_tok(rest);
    let comm = rest.trim();
    if pcpu.is_empty() || comm.is_empty() {
        return None;
    }
    let pid: i64 = pid_s.parse().ok()?;
    Some((pcpu.to_string(), pid, comm.to_string()))
}

async fn ps_top_cpu() -> Vec<(String, i64, String)> {
    run("ps", &["-eo", "pcpu=,pid=,comm=", "--sort=-pcpu"])
        .await
        .lines()
        .filter_map(parse_pcpu_line)
        .take(5)
        .collect()
}

/// Port of cpu.sh:228-233's `stuck()`: D/uninterruptible or Z/zombie
/// processes stuck for 20s+, excluding kthreadd (pid/ppid 2) and — for
/// zombies only — anything parented to `mango-bard` itself (cpu.sh's
/// `pgrep -x waybar`; a bar reaping its own dead module children is noise,
/// not signal — see the module doc above `stuck()` in the shell). ppid is
/// deliberately never returned: nothing downstream needs it, same as the
/// shell version blanking it before printing.
pub fn stuck(ps_text: &str, own_pids: &str) -> Vec<(String, i64, i64, String)> {
    let wb: std::collections::HashSet<&str> = own_pids.split_whitespace().collect();
    let mut out = Vec::new();
    for line in ps_text.lines() {
        let (state, rest) = take_tok(line);
        let (etimes_s, rest) = take_tok(rest);
        let (ppid_s, rest) = take_tok(rest);
        let (pid_s, rest) = take_tok(rest);
        let comm = rest.trim();
        if state.is_empty() || comm.is_empty() {
            continue;
        }
        if state != "D" && state != "Z" {
            continue;
        }
        let Ok(etimes) = etimes_s.parse::<i64>() else {
            continue;
        };
        if etimes < 20 {
            continue;
        }
        if ppid_s == "2" || pid_s == "2" {
            continue;
        }
        if state == "Z" && wb.contains(ppid_s) {
            continue;
        }
        let Ok(pid) = pid_s.parse::<i64>() else {
            continue;
        };
        out.push((state.to_string(), etimes, pid, comm.to_string()));
    }
    out
}

async fn ps_stuck() -> Vec<(String, i64, i64, String)> {
    let out = run("ps", &["-eo", "state=,etimes=,ppid=,pid=,comm="]).await;
    stuck(&out, &std::process::id().to_string())
}

// ------------------------------------------------------------------ build_tip

struct DetailInputs<'a> {
    total: i64,
    ncore: usize,
    percts: &'a [i64],
    gap: usize,
    loadavg: &'a str,
    freq_line: &'a str,
    policy_line: &'a str,
    temp_c: Option<i64>,
    top_ps: &'a [(String, i64, String)],
    stuck_rows: &'a [(String, i64, i64, String)],
}

/// ironbar's popup has no scroll widget, so a tip taller than the output
/// renders as an empty popup. These two caps keep the CPU tip inside one
/// screen whatever `ps` returns.
// T32: cpu_tip shares a popup (and its screen-height budget) with mem_tip
// under the sysload module — measured live (grim capture against the
// running bar, `ironbar bar <name> show-popup sysload` with a probe value
// injected via `ironbar var set`, bypassing mango-bard so the injected
// content isn't overwritten by a live refresh) that combined content
// above roughly 43-44 rendered lines makes the WHOLE popup fail to map,
// not just clip — there is no partial-render state to trust. 3/2 leaves
// real margin under that ceiling with mem_tip's own caps (memory.rs).
const MAX_TOP: usize = 3;
const MAX_STUCK: usize = 2;

/// Port of cpu.sh:371-438's tooltip build. Pure and fixture-testable —
/// everything forked or read from `/proc`/`sysfs` is gathered by
/// `refresh_detail` first.
fn build_tip(d: &DetailInputs) -> String {
    let mut tip = String::new();
    tip.push_str(&sect(&IC_LOAD.to_string(), "Load"));
    tip.push_str(&row(&format!(
        "{:>3}%  {}",
        d.total,
        bar(d.total, grade(d.total, 70, 90), 20)
    )));
    tip.push('\n');
    let mut la = d.loadavg.split_whitespace();
    let (a, b, c) = (
        la.next().unwrap_or("?"),
        la.next().unwrap_or("?"),
        la.next().unwrap_or("?"),
    );
    tip.push_str(&dim(&format!(
        "{a} / {b} / {c}  load average  ·  {} threads",
        d.ncore
    )));
    tip.push('\n');

    tip.push_str(&sect(&IC_CORES.to_string(), "Cores"));
    let (min, max) = d
        .percts
        .iter()
        .fold((100i64, 0i64), |(mn, mx), &p| (mn.min(p), mx.max(p)));
    tip.push_str(&row(&format!(
        "{}  {min}–{max}%",
        heatbar(d.percts, 70, 90, d.gap)
    )));
    tip.push('\n');
    // T32: the per-core text grid this popup used to print below the
    // heatbar is gone — it was a second, taller rendering of the same
    // per-core percentages the heatbar sparkline above already shows in
    // one line, and the sysload popup shares its screen-height budget
    // with mem_tip in the same popup.
    let temp = d.temp_c.map(format_temp).unwrap_or_default();
    let freq_temp = if temp.is_empty() {
        d.freq_line.to_string()
    } else {
        format!("{}  ·  {temp}", d.freq_line)
    };
    tip.push_str(&dim(&freq_temp));
    tip.push('\n');
    if !d.policy_line.is_empty() {
        tip.push_str(&dim(d.policy_line));
        tip.push('\n');
    }

    tip.push_str(&sect(&IC_TOP.to_string(), "Top now"));
    for (pcpu, pid, comm) in d.top_ps.iter().take(MAX_TOP) {
        let int_part: i64 = pcpu.split('.').next().unwrap_or("0").parse().unwrap_or(0);
        let meter = mono(&format!(
            "{} {pcpu:>5}%",
            bar(int_part, grade(int_part, 50, 80), 14)
        ));
        tip.push_str(&row(&format!(
            "{meter}  {} <span foreground=\"{C_DIM}\">{pid}</span>",
            esc(comm)
        )));
        tip.push('\n');
    }

    if !d.stuck_rows.is_empty() {
        tip.push_str(&sect(&IC_STUCK.to_string(), "Stuck"));
        for (state, etimes, pid, comm) in d.stuck_rows.iter().take(MAX_STUCK) {
            let what = if state == "D" {
                "uninterruptible"
            } else {
                "zombie"
            };
            tip.push_str(&row(&format!(
                "{} <span foreground=\"{C_DIM}\">{pid} · {what} · {}</span>",
                bad(&esc(comm)),
                hdur(*etimes)
            )));
            tip.push('\n');
        }
    }

    tip.trim_end_matches('\n').to_string()
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

    // ---- T7a additions, fixtures ported verbatim from cpu.sh's own `test`.

    #[test]
    fn core_labels_hybrid_splits_p_and_e_cores() {
        assert_eq!(
            core_labels(&[5_000_000, 5_000_000, 3_700_000, 3_700_000]),
            (
                3,
                vec![
                    "P0".to_string(),
                    "P1".to_string(),
                    "E2".to_string(),
                    "E3".to_string()
                ]
            )
        );
    }

    #[test]
    fn core_labels_uniform_cpu_has_no_split() {
        assert_eq!(
            core_labels(&[4_000_000, 4_000_000, 4_000_000, 4_000_000]),
            (
                0,
                vec![
                    "c0".to_string(),
                    "c1".to_string(),
                    "c2".to_string(),
                    "c3".to_string()
                ]
            )
        );
    }

    #[test]
    fn core_labels_no_cpufreq_falls_back_to_plain_labels() {
        assert_eq!(
            core_labels(&[0, 0]),
            (0, vec!["c0".to_string(), "c1".to_string()])
        );
    }

    #[test]
    fn cpu_freq_flat_cores_have_no_spread() {
        assert_eq!(
            cpu_freq("cpu MHz\t\t: 3000.000\ncpu MHz\t\t: 3000.000\n"),
            "3.00 GHz avg"
        );
    }

    #[test]
    fn cpu_freq_spread_appends_the_range() {
        assert_eq!(
            cpu_freq("cpu MHz\t\t: 800.000\ncpu MHz\t\t: 4200.000\n"),
            "2.50 GHz avg  ·  0.80–4.20 GHz"
        );
    }

    #[test]
    fn cpu_freq_empty_input_is_silent() {
        assert_eq!(cpu_freq(""), "");
    }

    #[test]
    fn cpu_policy_joins_only_present_parts() {
        assert_eq!(
            cpu_policy("performance", "", "0"),
            "performance  ·  turbo on"
        );
        assert_eq!(cpu_policy("", "", ""), "");
    }

    #[test]
    fn package_temp_picks_the_max_named_sensor_and_ignores_others() {
        let sensors = vec![
            ("nvme".to_string(), 90_000),
            ("coretemp".to_string(), 45_000),
            ("coretemp".to_string(), 52_000),
        ];
        assert_eq!(package_temp(&sensors), Some(52_000));
        assert_eq!(package_temp(&[("nvme".to_string(), 90_000)]), None);
    }

    // stuck(): fixture copied verbatim from cpu.sh's own `test`.
    #[test]
    fn stuck_matches_cpu_sh_selftest_fixture() {
        let ps = "Z 300 4242 5001 cpu.sh\n\
            Z 300 1 5002 orphan.sh\n\
            D 300 1 5003 blocked\n\
            D 300 2 5004 kworker\n\
            Z 5 4242 5005 quick.sh\n\
            D 300 4242 5006 net.sh\n";
        assert_eq!(
            stuck(ps, "4242 4243"),
            vec![
                ("Z".to_string(), 300, 5002, "orphan.sh".to_string()),
                ("D".to_string(), 300, 5003, "blocked".to_string()),
                ("D".to_string(), 300, 5006, "net.sh".to_string()),
            ]
        );
    }

    #[test]
    fn stuck_with_no_own_pids_does_not_swallow_every_zombie() {
        assert_eq!(
            stuck("Z 300 4242 5001 cpu.sh\n", ""),
            vec![("Z".to_string(), 300, 5001, "cpu.sh".to_string())]
        );
    }

    #[test]
    fn parse_pcpu_line_keeps_internal_spaces_in_comm() {
        assert_eq!(
            parse_pcpu_line("  12.3 4567 Isolated Web Co"),
            Some(("12.3".to_string(), 4567, "Isolated Web Co".to_string()))
        );
    }

    #[test]
    fn build_tip_opens_on_the_load_section() {
        let tip = build_tip(&DetailInputs {
            total: 12,
            ncore: 2,
            percts: &[10, 20],
            gap: 0,
            loadavg: "0.10 0.20 0.30 1/200 999",
            freq_line: "3.00 GHz avg",
            policy_line: "",
            temp_c: None,
            top_ps: &[],
            stuck_rows: &[],
        });
        // T-popup-vert: "CPU" moved out of the body into its own
        // `cpu_tip_title` ironvar — the body now opens on the Load section.
        assert!(tip.trim_start().starts_with("<span"));
        assert!(tip.contains("Load"));
        assert!(!tip.ends_with('\n'), "trailing newlines must be trimmed");
    }

    // ironbar has no scroll widget in a popup, so a tip taller than the
    // output renders as an empty popup. This guard fails if a later change
    // lets `ps` output length reach the tip again. The fixed sections cost
    // about 13 lines regardless of core count (the per-core text grid was
    // dropped — the heatbar sparkline already shows the same data in one
    // line), so the fixture keeps the core count small: what is under test
    // is the two list caps, not the size of the machine.
    #[test]
    fn build_tip_caps_the_process_lists() {
        let top: Vec<(String, i64, String)> = (0..50)
            .map(|i| ("9.9".to_string(), i, format!("proc{i}")))
            .collect();
        let stuck: Vec<(String, i64, i64, String)> = (0..50)
            .map(|i| ("D".to_string(), 300, i, format!("blocked{i}")))
            .collect();
        let tip = build_tip(&DetailInputs {
            total: 99,
            ncore: 2,
            percts: &[10, 20],
            gap: 0,
            loadavg: "9.0 8.0 7.0 1/200 999",
            freq_line: "3.00 GHz avg",
            policy_line: "performance",
            temp_c: Some(52_000),
            top_ps: &top,
            stuck_rows: &stuck,
        });
        let lines = tip.lines().count();
        assert!(lines < 20, "cpu tip grew to {lines} lines:\n{tip}");
        assert!(tip.contains("proc2"), "3 top rows expected");
        assert!(!tip.contains("proc3"), "4th top row must be dropped");
        assert!(tip.contains("blocked1"), "2 stuck rows expected");
        assert!(!tip.contains("blocked2"), "3rd stuck row must be dropped");
    }
}
