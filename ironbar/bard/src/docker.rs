//! Docker container pill: running count in the bar, every container grouped
//! by compose project in the popup. Ports src/waybar/scripts/docker.sh
//! (`custom/docker`) — see IRONBAR.md T6b for the design.
//!
//! `refresh()` forks exactly one process (`docker ps -a --format ...`,
//! docker.sh:107's own format string) — the `awk`×4/`sort`/`wc -l` pipeline
//! disappears into plain Rust parsing. `docker events` is a
//! [`crate::net::MonitorChild`] like every other T3+ event stream, but with
//! its noise filtered server-side (`--filter`): this machine's
//! `headroom-sidecar` healthcheck fires an `exec_create`/`exec_start`/
//! `exec_die` triple every probe (~6/min unfiltered, measured live), which
//! the filter set below drops before the daemon ever wakes for it (0
//! lines/27s idle, measured). `health_status` stays in the filter — it is
//! what flips the dot colour and the `warning` class, and docker only emits
//! it on an actual health *transition*.
//!
//! Not ported: docker.sh's `waybar-docker-tip` / `tip_bucket` cache
//! (docker.sh:130-131) — that cache exists only because the script re-execs
//! every 10s; a long-lived collector refreshing purely on events has no such
//! problem (same reasoning as T5's Power::refresh not needing battery.sh's
//! RAPL-sample cache file).

use crate::cmd::CMD_TIMEOUT;
use crate::mango::CLASS_PREFIX;
use crate::net::MonitorChild;
use crate::tooltip::{barico_label, dim, dot, esc, mono, projhdr, set_titled, C_DIM};
use crate::vars::Vars;
use tokio::process::Command;
use tokio::time::timeout;

/// T16 retracts T8b's stated root cause. T8b tried nf-linux-docker (whale,
/// U+F308), saw it render as CJK tofu, and concluded this ironbar/GTK4 build
/// cannot rasterize that PUA codepoint — then fell back to U+F13B (a generic
/// container/box badge, not a whale). The actual cause: `fc-list
/// ":charset=f308" family` also lists `IBM Plex Sans TC`, and style.css's
/// icon font stack puts `"IBM Plex Sans"` ahead of `"JetBrainsMono Nerd
/// Font"` — fontconfig matched the CJK face for that one codepoint, not a
/// missing glyph in the Nerd Font. U+F0868 (nf-md-docker, Plane-15 PUA) has
/// no such competing claimant (`fc-list ":charset=f0868" family` returns
/// only the Nerd Font), so it is the real whale. See IRONBAR.md's T16 entry.
const IC_DOCKER: char = '\u{f0868}';

const FORMAT: &str = r#"{{.Names}}|{{.State}}|{{.Status}}|{{.Image}}|{{.Label "com.docker.compose.project"}}|{{.Label "com.docker.compose.service"}}"#;

fn class_key(module: &str) -> String {
    format!("{CLASS_PREFIX}{module}")
}

fn docker_bin() -> String {
    std::env::var("MANGO_DOCKER").unwrap_or_else(|_| "docker".to_string())
}

// ---------------------------------------------------------------- parsing

#[derive(Debug, Clone, PartialEq)]
struct Row {
    name: String,
    state: String,
    status: String,
    image: String,
    project: String,
    service: String,
}

/// docker.sh:17's `$FORMAT`, one row per line, `|`-delimited.
fn parse_rows(raw: &str) -> Vec<Row> {
    raw.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut f = l.splitn(6, '|');
            Row {
                name: f.next().unwrap_or("").to_string(),
                state: f.next().unwrap_or("").to_string(),
                status: f.next().unwrap_or("").to_string(),
                image: f.next().unwrap_or("").to_string(),
                project: f.next().unwrap_or("").to_string(),
                service: f.next().unwrap_or("").to_string(),
            }
        })
        .collect()
}

/// docker.sh:35-38 `grouped()`: stable sort by compose project, empty
/// project sorts last (key `"~"`, C-locale sorts after any lowercase/
/// digit/hyphen project name). `Vec::sort_by` is a stable sort, so this
/// needs no explicit tie-break — order within a project is preserved for
/// free, same guarantee `sort -s` gives the shell.
fn grouped(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort_by(|a, b| sort_key(a).cmp(sort_key(b)));
    rows
}

fn sort_key(r: &Row) -> &str {
    if r.project.is_empty() {
        "~"
    } else {
        &r.project
    }
}

/// docker.sh:42-48 `dot_class()`: good/warn/bad/dim, in that priority — an
/// unhealthy container outranks a restarting one, health-starting outranks
/// plain running.
fn dot_class(state: &str, status: &str) -> &'static str {
    if status.contains("(unhealthy)") {
        "bad"
    } else if state == "restarting" || status.contains("(health: starting)") {
        "warn"
    } else if state == "running" {
        "good"
    } else {
        "dim"
    }
}

/// docker.sh:59-61 `health_mark()`.
fn health_mark(status: &str) -> &'static str {
    if status.contains("(healthy)") {
        " \u{2713}"
    } else {
        ""
    }
}

/// docker.sh:115-117: running/stopped counts and distinct compose-project
/// count.
fn counts(rows: &[Row]) -> (usize, usize, usize) {
    let running = rows.iter().filter(|r| r.state == "running").count();
    let stopped = rows
        .iter()
        .filter(|r| r.state != "running" && !r.state.is_empty())
        .count();
    let mut seen: Vec<&str> = Vec::new();
    for r in rows {
        if !r.project.is_empty() && !seen.contains(&r.project.as_str()) {
            seen.push(&r.project);
        }
    }
    (running, stopped, seen.len())
}

/// docker.sh:119-121: module-level warning flag — a restarting or unhealthy
/// row flips it, a plain exited/starting one does not.
fn class_for(rows: &[Row]) -> &'static str {
    let flagged = rows
        .iter()
        .any(|r| r.state == "restarting" || r.status.contains("(unhealthy)"));
    if flagged {
        "warning"
    } else {
        "normal"
    }
}

// ------------------------------------------------------------------ popup

/// Writes the pending "N stopped" line for the project that just ended and
/// resets the counter. Writes nothing when the project had no stopped rows.
fn flush_stopped(tip: &mut String, count: &mut usize) {
    if *count == 0 {
        return;
    }
    tip.push_str(&dim(&format!("{count} stopped")));
    tip.push('\n');
    *count = 0;
}

/// docker.sh:133-153's `TIP=$(...)` build. `PREV` starts as `""` in the
/// shell (not "no group yet") — replicated verbatim here via
/// `prev: String`, including the edge case that follows from it: if every
/// row is unlabeled (`project == ""` for the whole machine), the very first
/// group's header is suppressed too, since `"" != ""` is false. That is the
/// original script's own behaviour, not a port bug.
///
/// ironbar's popup has no scroll widget, so a tip taller than the output
/// renders as an empty popup. A machine with many old compose stacks has
/// far more stopped containers than running ones, and a stopped container
/// says nothing a count does not. So each project collapses its stopped
/// rows into one line. A *restarting* row is not collapsed: it is what
/// turns the pill orange (`class_for`), so the popup must name it.
fn build_tip(rows: &[Row], stopped: usize, projects: usize) -> String {
    let mut tip = String::new();
    let mut prev = String::new();
    let mut group_stopped = 0usize;
    for r in grouped(rows.to_vec()) {
        if r.name.is_empty() {
            continue;
        }
        if r.project != prev {
            flush_stopped(&mut tip, &mut group_stopped);
            let label = if r.project.is_empty() {
                "standalone"
            } else {
                &r.project
            };
            tip.push_str(&projhdr(&esc(label)));
            prev = r.project.clone();
        }
        if r.state != "running" && r.state != "restarting" {
            group_stopped += 1;
            continue;
        }
        let label = if r.service.is_empty() {
            &r.name
        } else {
            &r.service
        };
        let class = dot_class(&r.state, &r.status);
        let status_health = format!("{}{}", r.status, health_mark(&r.status));
        // docker.sh:147 escapes THEN pads to 24 — the padding measures the
        // already-escaped string, not the raw one. Replicated as written.
        let padded = format!("{:<24}", esc(&status_health));
        tip.push_str(&format!(
            "{}  {} <span foreground=\"{C_DIM}\">{}</span>\n",
            mono(&format!("{} {padded}", dot(class))),
            esc(label),
            esc(&r.image),
        ));
    }
    flush_stopped(&mut tip, &mut group_stopped);

    tip.push('\n');
    let plural = if projects == 1 { "" } else { "s" };
    tip.push_str(&dim(&format!(
        "{stopped} stopped  \u{b7}  {projects} project{plural}"
    )));

    tip.trim_end_matches('\n').to_string()
}

// ----------------------------------------------------------------- Docker

pub struct Docker {
    pub events_mon: MonitorChild,
    last_line: Option<String>,
}

impl Docker {
    pub fn new() -> Self {
        Self {
            // D1 (IRONBAR.md T6b): filtered server-side so the daemon never
            // wakes for exec probes / client noise. `health_status` stays in
            // — it only fires on a real health transition.
            events_mon: MonitorChild::new(
                "docker",
                &[
                    "events",
                    "--format",
                    "{{.Action}}",
                    "--filter",
                    "type=container",
                    "--filter",
                    "event=start",
                    "--filter",
                    "event=die",
                    "--filter",
                    "event=stop",
                    "--filter",
                    "event=create",
                    "--filter",
                    "event=destroy",
                    "--filter",
                    "event=restart",
                    "--filter",
                    "event=kill",
                    "--filter",
                    "event=pause",
                    "--filter",
                    "event=unpause",
                    "--filter",
                    "event=health_status",
                ],
            ),
            last_line: None,
        }
    }

    /// Raw-line dedup only — the `--filter` set above is the noise gate, so
    /// (unlike audio.rs's `event_matches`) every line that gets this far is
    /// already regrade-worthy.
    pub fn ingest_line(&mut self, line: &str) -> bool {
        if self.last_line.as_deref() == Some(line) {
            return false;
        }
        self.last_line = Some(line.to_string());
        true
    }

    /// docker.sh:107-159. The only thing that forks.
    pub async fn refresh(&mut self, vars: &mut Vars) {
        let output = timeout(
            CMD_TIMEOUT,
            Command::new(docker_bin())
                .args(["ps", "-a", "--format", FORMAT])
                .output(),
        )
        .await;
        let raw = match output {
            Ok(Ok(o)) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
            _ => {
                // daemon unreachable — empty text hides the module, same as
                // net.sh does mid-scan; no error pill, no tofu.
                vars.set("docker_text", "");
                set_titled(vars, "docker_tip", "Docker", String::new());
                // T29: `docker` shares `devload`'s node with `claude`/
                // `archupdate` now (`devload_module()`'s own doc comment)
                // — "" clears docker's own slot without touching theirs,
                // so it stays bare, not prefixed.
                vars.set(&class_key("devload#docker"), "");
                return;
            }
        };

        let rows = parse_rows(&raw);
        let (running, stopped, projects) = counts(&rows);
        vars.set(
            "docker_text",
            format!("{} {running}", barico_label(IC_DOCKER)),
        );
        set_titled(vars, "docker_tip", "Docker", build_tip(&rows, stopped, projects));
        // T29: prefixed so a real "warning"/"normal" collision with
        // `claude`'s or `archupdate`'s own value on the same shared node
        // can never happen — see cpu.rs's set_vars for the cross-talk
        // mechanism this guards against.
        vars.set(
            &class_key("devload#docker"),
            format!("dok-{}", class_for(&rows)),
        );
    }
}

impl Default for Docker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixture copied verbatim from docker.sh:71-75.
    const SAMPLE: &str = "crema-v16-mariadb-1|running|Up 3 minutes (healthy)|mariadb:11.8|crema-v16|mariadb\n\
crema-v16-redis-cache-1|running|Up 3 minutes|redis:alpine|crema-v16|redis-cache\n\
crema-develop-mariadb-1|running|Up 4 minutes (health: starting)|mariadb:11.8|crema-develop|mariadb\n\
old-postgres-1|exited|Exited (0) 2 days ago|postgres:15||\n\
loose-1|restarting|Restarting (1) 5 seconds ago|busybox||";

    #[test]
    fn grouped_sorts_projects_and_pushes_unlabeled_last() {
        let rows = parse_rows(SAMPLE);
        assert_eq!(rows.len(), 5, "docker.sh:78 expects 5 rows");
        let g = grouped(rows);
        assert_eq!(g[0].project, "crema-develop", "docker.sh:80");
        assert!(
            g[3].project.is_empty() && g[4].project.is_empty(),
            "docker.sh:79 — unlabeled containers pushed last"
        );
    }

    #[test]
    fn dot_class_matches_docker_sh_selftest_table() {
        assert_eq!(dot_class("running", "Up 3 minutes (healthy)"), "good");
        assert_eq!(dot_class("running", "Up 3 minutes (unhealthy)"), "bad");
        assert_eq!(
            dot_class("running", "Up 4 minutes (health: starting)"),
            "warn"
        );
        assert_eq!(
            dot_class("restarting", "Restarting (1) 5 seconds ago"),
            "warn"
        );
        assert_eq!(dot_class("exited", "Exited (0) 2 days ago"), "dim");
    }

    #[test]
    fn health_mark_present_only_when_healthy() {
        assert_eq!(health_mark("Up 3m (healthy)"), " \u{2713}");
        assert_eq!(health_mark("Up 3m"), "");
    }

    #[test]
    fn counts_match_docker_sh_selftest_table() {
        let rows = parse_rows(SAMPLE);
        let (running, stopped, projects) = counts(&rows);
        assert_eq!(running, 3, "docker.sh:92");
        assert_eq!(stopped, 2);
        assert_eq!(projects, 2, "crema-v16 and crema-develop");
    }

    #[test]
    fn class_for_flags_restarting_or_unhealthy_only() {
        let rows = parse_rows(SAMPLE);
        assert_eq!(
            class_for(&rows),
            "warning",
            "docker.sh:96 — loose-1 is restarting"
        );
        let clean = parse_rows("a|running|Up 1 minute (healthy)|x|p|s");
        assert_eq!(class_for(&clean), "normal", "docker.sh:98");
    }

    #[test]
    fn ingest_line_dedups_identical_lines() {
        let mut d = Docker::new();
        assert!(d.ingest_line("start"));
        assert!(
            !d.ingest_line("start"),
            "identical line must not re-trigger"
        );
        assert!(d.ingest_line("die"));
    }

    #[test]
    fn build_tip_groups_by_project_with_a_standalone_header() {
        let rows = parse_rows(SAMPLE);
        let tip = build_tip(&rows, 2, 2);
        // T-popup-vert: the "Docker" title moved out of the body into its
        // own `docker_tip_title` ironvar (see genconfig.rs::popup()) — the
        // body now opens straight on the first project header.
        assert!(
            tip.trim_start().starts_with("<span"),
            "project header markup expected: {tip}"
        );
        assert!(tip.contains("crema-v16"));
        assert!(tip.contains("crema-develop"));
        assert!(
            tip.contains("standalone"),
            "unlabeled containers get this header"
        );
        assert!(tip.contains("2 stopped"));
        assert!(tip.contains("2 projects"));
        assert!(!tip.ends_with('\n'), "trailing newlines must be trimmed");
        // old-postgres-1 is exited, so it collapses into a count. loose-1 is
        // restarting, so it keeps its own row.
        assert!(!tip.contains("old-postgres-1"));
        assert!(tip.contains("loose-1"));
    }

    // ironbar has no scroll widget in a popup, so a tip taller than the
    // output renders as an empty popup. Stopped containers are what grow
    // without bound on a machine with old compose stacks, so this guard
    // fails if they ever get a line each again.
    #[test]
    fn build_tip_collapses_stopped_containers_per_project() {
        let mut raw = String::new();
        for p in 0..4 {
            raw.push_str(&format!("p{p}-web-1|running|Up 3 minutes|nginx|p{p}|web\n"));
            for c in 0..8 {
                raw.push_str(&format!(
                    "p{p}-old-{c}|exited|Exited (0) 2 days ago|busybox|p{p}|old{c}\n"
                ));
            }
        }
        let rows = parse_rows(&raw);
        assert_eq!(rows.len(), 36);
        let tip = build_tip(&rows, 32, 4);
        let lines = tip.lines().count();
        assert!(lines < 25, "docker tip grew to {lines} lines:\n{tip}");
        assert_eq!(
            tip.matches("8 stopped").count(),
            4,
            "each project needs its own collapsed count"
        );
        assert!(tip.contains("p0-web-1") || tip.contains("web"));
        assert!(!tip.contains("p0-old-3"), "stopped rows must be collapsed");
    }

    #[test]
    fn build_tip_singular_project_has_no_trailing_s() {
        let rows = parse_rows("a|running|Up 1 minute|x|solo|s");
        let tip = build_tip(&rows, 0, 1);
        assert!(tip.contains("1 project"));
        assert!(!tip.contains("1 projects"));
    }
}
