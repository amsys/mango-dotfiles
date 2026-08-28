//! Memory bar pill (T6a) plus the detail popup (T7a). Ports
//! src/waybar/scripts/memory.sh in full: `mem_kv()` (memory.sh:33) and the
//! pill's own percent/class/text lines (memory.sh:122-133) at T6a; swap
//! description, the cached DIMM table, page-fault rates and the RSS top
//! list (memory.sh's tooltip half) at T7a.
//!
//! Same split as cpu.rs: `refresh()` (the T6a pill path, on the wheel) stays
//! pure `/proc` reads with zero forks; `refresh_detail` runs only from the
//! control socket's `mem-detail` topic, poked from inside the popup itself
//! while it is open (IRONBAR.md T7 design D1/D2). Unlike cpu, memory needs
//! no prior sample for the *pill* — `refresh()` is correct on the very first
//! call — but the popup's page-fault rate does need one, held on `Memory`.

use crate::cmd::run;
use crate::mango::CLASS_PREFIX;
use crate::tooltip::{
    bad, bar, barico_label, dim, esc, good, grade, hcount, hkib, level_class, mono, row, sect,
    set_titled, C_DIM,
};
use crate::vars::Vars;
use std::collections::HashMap;

/// memory_alt glyph, memory.sh:22 (`ic_mem`).
/// T8b: U+F7A3 (Material Symbols "memory_alt") -> U+F1C0 (database/stack,
/// JetBrainsMono Nerd Font Font Awesome) — GTK4 cannot correctly rasterize
/// Material Symbols Rounded's variable font on this system; see IRONBAR.md's
/// T8b entry. Verified by loading it in a real ironbar instance and
/// screenshotting the result, not just pango-view (see cpu.rs's IC_CPU
/// comment for why pango-view alone isn't sufficient evidence).
/// T19: U+F1C0 -> U+F035B (md-memory) — one-icon-family sweep (IRONBAR.md
/// T19); distinct from `IC_HW` below (md-chip) so the bar pill and the
/// popup's "Hardware" section header don't share a glyph.
const IC_MEM: char = '\u{f035b}';

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
    // T28: digit dropped entirely — see cpu.rs's set_vars for the gauge/
    // `#level`-slot/width-target reasoning, identical here.
    //
    // T29: both slot keys move under `sysload`, both values gain a
    // `mem-`/`ml` prefix — see cpu.rs's set_vars for why the prefix has to
    // sit here (at the point of writing to the shared node) rather than
    // inside `class_for` itself.
    vars.set("mem_text", barico_label(IC_MEM));
    vars.set(
        &class_key("sysload#mem"),
        format!("mem-{}", class_for(pct, swap_pct)),
    );
    vars.set(&class_key("sysload#memlevel"), level_class("ml", pct));
}

/// pgfault, pgmajfault, `/proc/uptime` seconds — the previous sample
/// `rate()` needs to turn since-boot counters into a per-second figure.
type VmSample = (i64, i64, i64);

#[derive(Default)]
pub struct Memory {
    prev_vmstat: Option<VmSample>,
    /// Parsed once from the DMI cache and kept — hardware does not change
    /// at runtime, so there is nothing to invalidate (D6). `None` until the
    /// first `refresh_detail` call attempts it.
    dimms_cache: Option<Vec<(String, String, String, String)>>,
}

impl Memory {
    pub fn new() -> Self {
        Self::default()
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

    /// T7a: builds `mem_tip` for the detail popup. Called only from the
    /// control socket's `mem-detail` topic — see cpu.rs's `refresh_detail`
    /// doc for the same lazy-popup reasoning (IRONBAR.md T7 D1/D2).
    pub async fn refresh_detail(&mut self, vars: &mut Vars) {
        let meminfo_text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let kv = mem_kv(&meminfo_text);
        let get = |k: &str| kv.get(k).copied().unwrap_or(0);
        let (total, avail) = (get("MemTotal"), get("MemAvailable"));
        let pct = pct_used(total - avail, total);
        let (swap_total, swap_free) = (get("SwapTotal"), get("SwapFree"));

        let vmstat_text = std::fs::read_to_string("/proc/vmstat").unwrap_or_default();
        let vm = vm_kv(&vmstat_text);
        let pgfault = vm.get("pgfault").copied().unwrap_or(0);
        let pgmajfault = vm.get("pgmajfault").copied().unwrap_or(0);
        let now = uptime_secs();
        let (min_rate, maj_rate) = match self.prev_vmstat {
            Some((pf, pmf, pt)) => (rate(pgfault, pf, now - pt), rate(pgmajfault, pmf, now - pt)),
            None => (0, 0),
        };
        self.prev_vmstat = Some((pgfault, pgmajfault, now));

        let swaps_text = std::fs::read_to_string("/proc/swaps").unwrap_or_default();

        if self.dimms_cache.is_none() {
            let dmi_text = std::fs::read_to_string(dmi_cache_path()).unwrap_or_default();
            self.dimms_cache = Some(dimms(&dmi_text));
        }
        let dimm_rows = self.dimms_cache.clone().unwrap_or_default();

        let top_rss = ps_top_rss().await;

        let tip = build_tip(&DetailInputs {
            pct,
            total,
            avail,
            cached: get("Cached"),
            buffers: get("Buffers"),
            shmem: get("Shmem"),
            dirty: get("Dirty"),
            writeback: get("Writeback"),
            swap_total,
            swap_free,
            swap_desc: swap_desc(&swaps_text),
            min_rate,
            maj_rate,
            pgfault,
            pgmajfault,
            top_rss: &top_rss,
            dimm_rows: &dimm_rows,
        });
        set_titled(vars, "mem_tip", "Memory", tip);
    }
}

fn uptime_secs() -> i64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_string))
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as i64)
        .unwrap_or(0)
}

fn dmi_cache_path() -> String {
    std::env::var("MANGO_DMI_CACHE").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.cache/mango-meminfo")
    })
}

// ---------------------------------------------------------------- T7a icons

const IC_RAM: char = '\u{f035b}'; // md-memory, memory.sh:23
const IC_SWAP: char = '\u{f04e1}'; // md-swap_horizontal, memory.sh:24
const IC_FAULT: char = '\u{f05d6}'; // md-alert_circle_outline, memory.sh:25
const IC_TOP: char = '\u{f0279}'; // md-format_list_bulleted, memory.sh:26
const IC_HW: char = '\u{f061a}'; // md-chip, memory.sh:27

/// Port of memory.sh:36's `vm_kv()`: `pgfault`/`pgmajfault` totals out of
/// `/proc/vmstat` — every other key is ignored (unlike [`mem_kv`], which
/// keeps everything).
pub fn vm_kv(proc_vmstat: &str) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    for line in proc_vmstat.lines() {
        let mut fields = line.split_whitespace();
        let Some(key) = fields.next() else { continue };
        if key != "pgfault" && key != "pgmajfault" {
            continue;
        }
        if let Some(v) = fields.next().and_then(|v| v.parse::<i64>().ok()) {
            out.insert(key.to_string(), v);
        }
    }
    out
}

/// Port of memory.sh:39-45's `rate()`: per-second delta between two counter
/// samples. A non-positive elapsed time or a counter that ran backwards
/// (a reset, not a real decrease) both clamp to 0 rather than going
/// negative or dividing by zero.
pub fn rate(now: i64, prev: i64, secs: i64) -> i64 {
    let d = now - prev;
    if secs <= 0 || d < 0 {
        return 0;
    }
    (d as f64 / secs as f64 + 0.5) as i64
}

/// Port of memory.sh:48-50's `swap_desc()`: the first `/proc/swaps` data
/// row as `"<basename> · <type>"`. Empty string with no swap configured.
pub fn swap_desc(proc_swaps: &str) -> String {
    for line in proc_swaps.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let Some(path) = fields.next() else { continue };
        let Some(kind) = fields.next() else { continue };
        let base = path.rsplit('/').next().unwrap_or(path);
        return format!("{base} · {kind}");
    }
    String::new()
}

/// Port of memory.sh:53-64's `dimms()`: one `(size, type, speed, part)`
/// tuple per *populated* `Memory Device` block in a cached `dmidecode -t
/// memory` dump (`/sys/firmware/dmi` needs root, so install-config.sh
/// caches this once — memory.sh:9-10). A block whose Size reads "No Module
/// Installed" or is blank is skipped, exactly as the awk state machine does.
pub fn dimms(dmi_text: &str) -> Vec<(String, String, String, String)> {
    let mut out = Vec::new();
    let mut have = false;
    let (mut size, mut kind, mut speed) = (String::new(), String::new(), String::new());
    for raw in dmi_text.lines() {
        let line = raw.trim_start_matches('\t');
        if line == "Memory Device" {
            size.clear();
            kind.clear();
            speed.clear();
            have = true;
            continue;
        }
        if !have {
            continue;
        }
        if let Some(v) = line.strip_prefix("Size:") {
            size = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Type:") {
            kind = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Configured Memory Speed:") {
            speed = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Part Number:") {
            let part = v.trim().to_string();
            if !size.is_empty() && !size.contains("No Module") {
                out.push((size.clone(), kind.clone(), speed.clone(), part));
            }
            have = false;
        }
    }
    out
}

// -------------------------------------------------------- ps/top processes

fn take_tok(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    s.split_at(end)
}

fn parse_rss_line(line: &str) -> Option<(i64, i64, String)> {
    let (rss_s, rest) = take_tok(line);
    let (pid_s, rest) = take_tok(rest);
    let comm = rest.trim();
    if comm.is_empty() {
        return None;
    }
    let rss: i64 = rss_s.parse().ok()?;
    let pid: i64 = pid_s.parse().ok()?;
    Some((rss, pid, comm.to_string()))
}

async fn ps_top_rss() -> Vec<(i64, i64, String)> {
    run("ps", &["-eo", "rss=,pid=,comm=", "--sort=-rss"])
        .await
        .lines()
        .filter_map(parse_rss_line)
        .take(5)
        .collect()
}

// ------------------------------------------------------------------ build_tip

struct DetailInputs<'a> {
    pct: i64,
    total: i64,
    avail: i64,
    cached: i64,
    buffers: i64,
    shmem: i64,
    dirty: i64,
    writeback: i64,
    swap_total: i64,
    swap_free: i64,
    swap_desc: String,
    min_rate: i64,
    maj_rate: i64,
    pgfault: i64,
    pgmajfault: i64,
    top_rss: &'a [(i64, i64, String)],
    dimm_rows: &'a [(String, String, String, String)],
}

/// Port of memory.sh:153-203's tooltip build. Pure and fixture-testable —
/// everything forked or read from `/proc`/the DMI cache is gathered by
/// `refresh_detail` first.
fn build_tip(d: &DetailInputs) -> String {
    let mut tip = String::new();
    tip.push_str(&sect(&IC_RAM.to_string(), "RAM"));
    tip.push_str(&row(&format!(
        "{:>3}%  {}",
        d.pct,
        bar(d.pct, grade(d.pct, 75, 90), 20)
    )));
    tip.push('\n');
    tip.push_str(&row(&format!(
        "{} used of {}  ·  {} available",
        hkib(d.total - d.avail),
        hkib(d.total),
        hkib(d.avail)
    )));
    tip.push('\n');
    tip.push_str(&dim(&format!(
        "{} cached  ·  {} buffers  ·  {} shared",
        hkib(d.cached),
        hkib(d.buffers),
        hkib(d.shmem)
    )));
    tip.push('\n');
    tip.push_str(&dim(&format!(
        "{} dirty  ·  {} in writeback",
        hkib(d.dirty),
        hkib(d.writeback)
    )));
    tip.push('\n');

    tip.push_str(&sect(&IC_SWAP.to_string(), "Swap"));
    if d.swap_total > 0 {
        let swap_pct = pct_used(d.swap_total - d.swap_free, d.swap_total);
        tip.push_str(&row(&format!(
            "{:>3}%  {}",
            swap_pct,
            bar(swap_pct, grade(swap_pct, 20, 50), 20)
        )));
        tip.push('\n');
        tip.push_str(&row(&format!(
            "{} used of {}",
            hkib(d.swap_total - d.swap_free),
            hkib(d.swap_total)
        )));
        tip.push('\n');
        tip.push_str(&dim(&d.swap_desc));
        tip.push('\n');
    } else {
        tip.push_str(&dim("none configured"));
        tip.push('\n');
    }

    tip.push_str(&sect(&IC_FAULT.to_string(), "Page faults"));
    let major = if d.maj_rate > 0 {
        bad(&format!("{}/s major", d.maj_rate))
    } else {
        good("no major faults")
    };
    tip.push_str(&row(&format!("{}/s minor  ·  {major}", hcount(d.min_rate))));
    tip.push('\n');
    tip.push_str(&dim(&format!(
        "{} minor, {} major since boot",
        hcount(d.pgfault),
        hcount(d.pgmajfault)
    )));
    tip.push('\n');

    tip.push_str(&sect(&IC_TOP.to_string(), "Top by RSS"));
    let top_rss_max = d
        .top_rss
        .iter()
        .map(|(rss, _, _)| *rss)
        .max()
        .unwrap_or(1)
        .max(1);
    for (rss, pid, comm) in d.top_rss {
        let rpct = pct_used(*rss, top_rss_max);
        let share = pct_used(*rss, d.total);
        let meter = mono(&format!(
            "{} {:>9}",
            bar(rpct, grade(share, 10, 25), 14),
            hkib(*rss)
        ));
        tip.push_str(&row(&format!(
            "{meter}  {} <span foreground=\"{C_DIM}\">{pid}</span>",
            esc(comm)
        )));
        tip.push('\n');
    }

    if !d.dimm_rows.is_empty() {
        tip.push_str(&sect(&IC_HW.to_string(), "Hardware"));
        for (size, kind, speed, part) in d.dimm_rows {
            tip.push_str(&row(&esc(&format!("{size} {kind}  ·  {speed}"))));
            tip.push('\n');
            tip.push_str(&dim(&esc(part)));
            tip.push('\n');
        }
    }

    tip.trim_end_matches('\n').to_string()
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

    // ---- T7a additions, fixtures ported verbatim from memory.sh's own `test`.

    #[test]
    fn vm_kv_picks_only_the_two_fault_counters() {
        let vm = vm_kv("pgfault 1000\npgmajfault 40\nnr_dirty 3\n");
        assert_eq!(vm.get("pgfault"), Some(&1000));
        assert_eq!(vm.get("pgmajfault"), Some(&40));
        assert_eq!(vm.len(), 2, "nr_dirty must not appear");
    }

    #[test]
    fn rate_matches_memory_sh_selftest_clamps() {
        assert_eq!(rate(1000, 400, 2), 300);
        assert_eq!(rate(10, 1000, 2), 0, "counter reset must clamp");
        assert_eq!(rate(10, 1, 0), 0, "zero elapsed must clamp");
    }

    #[test]
    fn hkib_used_matches_memory_sh_selftest_fixture() {
        // MemTotal - MemAvailable from the shell fixture: 48932736 - 30000000
        assert_eq!(hkib(48_932_736 - 30_000_000), "18.1 GiB");
    }

    #[test]
    fn swap_desc_matches_memory_sh_selftest_fixture() {
        let swaps =
            "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n/swapfile file 4194300 0 -1\n";
        assert_eq!(swap_desc(swaps), "swapfile · file");
    }

    #[test]
    fn dimms_skips_unpopulated_slots_and_matches_memory_sh_selftest_fixture() {
        let dmi = "Memory Device\n\
            \tSize: No Module Installed\n\
            \tType: Unknown\n\
            \tConfigured Memory Speed: Unknown\n\
            \tPart Number: Not Specified\n\
            Memory Device\n\
            \tSize: 32 GB\n\
            \tType: DDR5\n\
            \tType Detail: Synchronous\n\
            \tConfigured Memory Speed: 5600 MT/s\n\
            \tPart Number: CT32G56C46S5\n";
        assert_eq!(
            dimms(dmi),
            vec![(
                "32 GB".to_string(),
                "DDR5".to_string(),
                "5600 MT/s".to_string(),
                "CT32G56C46S5".to_string()
            )]
        );
    }

    #[test]
    fn parse_rss_line_keeps_internal_spaces_in_comm() {
        assert_eq!(
            parse_rss_line("  12345 6789 Web Content"),
            Some((12345, 6789, "Web Content".to_string()))
        );
    }

    #[test]
    fn build_tip_hides_hardware_section_when_dmi_cache_is_empty() {
        let tip = build_tip(&DetailInputs {
            pct: 39,
            total: 48_932_736,
            avail: 30_000_000,
            cached: 20_000_000,
            buffers: 500_000,
            shmem: 1_000_000,
            dirty: 4096,
            writeback: 0,
            swap_total: 0,
            swap_free: 0,
            swap_desc: String::new(),
            min_rate: 0,
            maj_rate: 0,
            pgfault: 1000,
            pgmajfault: 40,
            top_rss: &[],
            dimm_rows: &[],
        });
        // T-popup-vert: "Memory" moved out of the body into its own
        // `mem_tip_title` ironvar — the body now opens on the RAM section.
        assert!(tip.trim_start().starts_with("<span"));
        assert!(tip.contains("none configured"));
        assert!(tip.contains("no major faults"));
        assert!(!tip.contains("Hardware"));
        assert!(!tip.ends_with('\n'), "trailing newlines must be trimmed");
    }

    #[test]
    fn build_tip_shows_hardware_section_when_dmi_cache_is_present() {
        let tip = build_tip(&DetailInputs {
            pct: 39,
            total: 48_932_736,
            avail: 30_000_000,
            cached: 20_000_000,
            buffers: 500_000,
            shmem: 1_000_000,
            dirty: 4096,
            writeback: 0,
            swap_total: 4_194_300,
            swap_free: 4_194_300,
            swap_desc: "swapfile · file".to_string(),
            min_rate: 300,
            maj_rate: 5,
            pgfault: 1000,
            pgmajfault: 40,
            top_rss: &[(1_048_576, 4242, "kitty".to_string())],
            dimm_rows: &[(
                "32 GB".to_string(),
                "DDR5".to_string(),
                "5600 MT/s".to_string(),
                "CT32G56C46S5".to_string(),
            )],
        });
        assert!(tip.contains("Hardware"));
        assert!(tip.contains("CT32G56C46S5"));
        assert!(
            tip.contains("5/s major"),
            "major fault rate should be bad-graded"
        );
        assert!(tip.contains("kitty"));
        assert!(tip.contains("4242"));
    }
}
