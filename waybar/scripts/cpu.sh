#!/bin/sh
# CPU indicator for waybar, with a per-core / hot-process / recent-history tooltip.
#
# Replaces the built-in `cpu` module: waybar's tooltip is a fixed string and
# cannot show which process is responsible, which cores are pinned, or what was
# hammering the machine ten minutes ago. The last one comes from atop, whose
# service is already logging every 10 minutes, and its parseable output names the
# top CPU consumer per sample, which is exactly the "what just froze my laptop"
# answer, with no need to decode atop's binary format by hand.
#
#   cpu.sh          custom/cpu exec — emit JSON
#   cpu.sh test     assert the parsers against canned input
set -u

. "$(dirname "$0")/tooltip.sh"

STATE="${XDG_RUNTIME_DIR:-/tmp}/waybar-cpu"
ATOP_DIR="${MANGO_ATOP_DIR:-/var/log/atop}"
CPU_SYS="${MANGO_CPU_SYS:-/sys/devices/system/cpu}"   # overridden by the self-check

# atopsar has to walk the whole day's log — 1.3s on a 17 MB file — which stalled
# waybar's main loop every interval and made the tooltip feel sticky. The log
# only gains a sample every 10 minutes, so it is read in the background into a
# cache and the tooltip renders from whatever is there.
ATOP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-cpu-atop"
ATOP_TTL=300

ic_cpu() { printf '\xee\x8c\xa2'; }  # memory (the chip glyph)  U+E322

IC_LOAD='󰓅'   # md-speedometer          U+F04C5
IC_CORES='󰘚'  # md-chip                 U+F061A
IC_TOP='󰉹'    # md-format_list_bulleted U+F0279
IC_STUCK='󰀪'  # md-alert                U+F002A
IC_HIST='󰄉'   # md-clock_fast/history   U+F0109

# ---------------------------------------------------------------- primitives

# "cpu <total> <idle>" per line, one for the aggregate and one per core.
snapshot() {
	awk '/^cpu[0-9]* /{ t = 0; for (i = 2; i <= NF; i++) t += $i; print $1, t, $5 + $6 }' "${1:-/proc/stat}"
}

# busy percentage per key, from two snapshots. Keys present in only one sample
# (a core coming online) are skipped rather than reported as 100%.
deltas() { # prev-file, cur-file
	awk 'NR == FNR { t[$1] = $2; i[$1] = $3; next }
	     ($1 in t) {
	         dt = $2 - t[$1]; di = $3 - i[$1]
	         p = (dt > 0 ? (dt - di) * 100 / dt + 0.5 : 0)
	         # counters reset across suspend/resume — clamp instead of printing nonsense
	         if (p < 0) p = 0; if (p > 100) p = 100
	         printf "%s %d\n", $1, p
	     }' "$1" "$2"
}

# `atop -P PRC` records -> "15:53<TAB>waybar<TAB>47", the top process per sample.
#
#   PRC host epoch date TIME IVAL pid (NAME) STATE hz UTIME STIME ... tgid ISPROC ...
#    $1   $2    $3   $4   $5   $6  $7   $8    f1   f2   f3    f4        f11   f12
#
# `atopsar -O` was the obvious source, but it prints two kinds of row it gives no
# way to tell apart from the ones wanted. Processes that *exited* during a sample
# come from atopacctd's accounting records, so their whole-lifetime CPU lands in
# the interval they died in — a two-hour claude session reads as 158%. And the
# per-thread records are mixed in among the processes, letting a helper thread
# outrank its own parent. STATE and ISPROC filter both out here.
#
# The name is paren-matched rather than read as $8: names contain spaces
# ("Bun Pool 0"), and a second, empty () field appears later in the record.
# Tab-separated output for the same reason.
atop_top() {
	awk '
	$1 == "RESET" { skip = 1; next }   # sample after -b: counters are since boot
	$1 == "SEP"   { skip = 0; next }
	$1 != "PRC" || skip { next }
	# atop also prints screen-style summaries starting "PRC | sys ...", whose $6 is
	# a word — awk would compare it as a string, pass the numeric guards below and
	# then divide by zero, killing the refresh and truncating the cache. Every real
	# record has a timestamp here and no summary line does.
	$5 !~ /^[0-9][0-9]:[0-9][0-9]:[0-9][0-9]$/ { next }
	{
		tm = substr($5, 1, 5); iv = $6
		rest = substr($0, index($0, "(") + 1)
		j = index(rest, ") "); name = substr(rest, 1, j - 1)
		split(substr(rest, j + 2), f, " ")
		if (j <= 0 || f[1] == "E" || f[12] != "y" || iv <= 0 || f[2] <= 0) next
		pct = int((f[3] + f[4]) * 100 / (f[2] * iv) + 0.5)
		if (tm != ptm) { flush(); ptm = tm; best = -1 }
		if (pct > best) { best = pct; bname = name }
	}
	# ptm is empty only before the first surviving record, so a sample whose rows
	# were all filtered away never advances it and never flushes a blank row.
	function flush() { if (ptm != "") printf "%s\t%s\t%d\n", ptm, bname, best }
	END { flush() }'
}

# Hybrid CPUs (Intel P/E, ARM big.LITTLE) interleave two kinds of core in one
# list, and "which kind is busy" is most of what a per-core view is for — 100% on
# an E-core and 100% on a P-core are not the same event. Derived from cpufreq
# rather than hardcoded per machine: the fastest max-frequency present is P,
# anything slower is E. A uniform CPU (or one with no cpufreq at all, e.g. a VM)
# has no split to draw, so it falls back to plain c0..cN.
#
# Emits "<gap> <label>..." on one line — the gap is the 1-based index of the
# first E-core, which is exactly where heatbar should break the row, and folding
# it into this output keeps the whole thing to a single awk.
core_labels() { # ncore
	awk -v n="$1" -v base="$CPU_SYS" 'BEGIN {
		for (i = 0; i < n; i++) {
			p = base "/cpu" i "/cpufreq/cpuinfo_max_freq"
			v = ""
			f[i] = ((getline v < p) > 0) ? v + 0 : 0
			close(p)
			if (f[i] > top) top = f[i]
		}
		for (i = 0; i < n; i++)
			if (f[i] != top) { hybrid = 1; break }
		for (i = 0; i < n; i++) {
			if (!hybrid || top == 0) { lb[i] = "c" i; continue }
			lb[i] = (f[i] == top ? "P" : "E") i
			if (f[i] != top && !gap) gap = i + 1
		}
		printf "%d", gap + 0
		for (i = 0; i < n; i++) printf " %s", lb[i]
		printf "\n"
	}'
}

# One row per two cores, laid out in two columns. Done in a single awk rather
# than calling bar/grade per core: at 12 cores that would be 24 more forks on
# every waybar interval, and this module already runs every few seconds.
#
# Column-major, so the P-cores stay together at the top of the left column
# instead of being split across both. Prints its own row indent to match row().
coregrid() { # "p0 p1 ...", "l0 l1 ..."
	awk -v e="$C_EMPTY" -v g="$C_GOOD" -v y="$C_WARN" -v r="$C_BAD" -v d="$C_DIM" -v mf="$F_MONO" -v i3="$IND3" '
	function cell(i,   c, k, q, f, m) {
		c = (p[i] >= 90 ? r : p[i] >= 70 ? y : g)
		k = int(p[i] / 10 + 0.5); if (k > 10) k = 10; if (k < 0) k = 0
		for (q = 0; q < k; q++)  f = f "█"
		for (q = k; q < 10; q++) m = m "░"
		return sprintf("<span foreground=\"%s\">%3s</span> <span foreground=\"%s\">%s</span><span foreground=\"%s\">%s</span> %3d%%",
			d, lb[i], c, f, e, m, p[i])
	}
	BEGIN {
		n = split(ARGV[1], p, " "); split(ARGV[2], lb, " "); ARGC = 1
		rows = int((n + 1) / 2)
		# the whole row, joiner included, has to sit in one fixed-width run —
		# a proportional space between the columns drifts just like the digits do
		for (i = 1; i <= rows; i++)
			printf "%s<span font_family=\"%s\">%s%s</span>%s\n", i3, mf, cell(i),
				(i + rows <= n ? "   " cell(i + rows) : ""), i3
	}' "$1" "$2"
}

# GHz rather than a bare four-digit MHz count, plus the spread across cores: on a
# hybrid chip the average alone hides that the P-cores are boosting while the
# E-cores sit at their floor. The spread is only worth printing when the cores
# actually disagree — otherwise it reads as a pointless "3.00–3.00 GHz".
cpu_freq() {
	awk '/^cpu MHz/ {
		s += $4; n++
		if (mn == 0 || $4 < mn) mn = $4
		if ($4 > mx) mx = $4
	}
	END {
		if (!n) exit
		printf "%.2f GHz avg", s / n / 1000
		if (mx - mn >= 50) printf "  ·  %.2f–%.2f GHz", mn / 1000, mx / 1000
	}' "${1:-/proc/cpuinfo}"
}

# Why the frequency is where it is. Every part is optional — AMD exposes no
# energy_performance_preference, and a kernel without intel_pstate has no
# no_turbo — so each is appended only if its file actually read.
cpu_policy() {
	gov= epp= nt=
	read -r gov < "$CPU_SYS/cpu0/cpufreq/scaling_governor" 2> /dev/null
	read -r epp < "$CPU_SYS/cpu0/cpufreq/energy_performance_preference" 2> /dev/null
	read -r nt < "$CPU_SYS/intel_pstate/no_turbo" 2> /dev/null
	out="$gov"
	[ -n "$epp" ] && out="${out:+$out  ·  }$epp"
	case "$nt" in
	0) out="${out:+$out  ·  }turbo on" ;;
	1) out="${out:+$out  ·  }turbo off" ;;
	esac
	printf '%s' "$out"
}

# True when the cache is missing or older than ATOP_TTL.
atop_stale() {
	[ -s "$ATOP_CACHE" ] || return 0
	[ $(($(date +%s) - $(stat -c %Y "$ATOP_CACHE" 2> /dev/null || echo 0))) -ge "$ATOP_TTL" ]
}

# Refresh in the background, detached from waybar's pipe — a child holding stdout
# open would make waybar wait for it. Only the last ~70 minutes are read: a
# whole-day -P PRC pass emits 135 MB and takes 2.8s, against 0.27s windowed, and
# only the last six samples are ever shown.
atop_refresh() {
	mkdir "$ATOP_CACHE.lock" 2> /dev/null || return 0   # a refresh is already running
	BEGIN=$(date -d '-70 min' +%H:%M)
	# before 01:10 that wraps into yesterday, which today's log does not contain
	[ "$BEGIN" \> "$(date +%H:%M)" ] && BEGIN=00:00
	(
		atop -P PRC -r "$1" -b "$BEGIN" 2> /dev/null |
			atop_top | tail -6 > "$ATOP_CACHE.tmp"
		# tail always succeeds, so an atop failure would otherwise install an
		# empty cache and suppress the "reading…" placeholder.
		[ -s "$ATOP_CACHE.tmp" ] && mv "$ATOP_CACHE.tmp" "$ATOP_CACHE"
		rmdir "$ATOP_CACHE.lock"
	) > /dev/null 2>&1 < /dev/null &
}

# D = uninterruptible sleep (stuck on IO or a wedged driver), Z = zombie.
# Kernel threads (parented to kthreadd, pid 2) sit in D as a matter of course —
# i915's flip worker is there permanently — so they are excluded rather than
# filtered by name, and only userspace stuck for 20s+ counts.
#
# waybar's own dead modules are excluded too. waybar SIGTERMs every module script
# on reload and never waitpid()s it, so each reload leaves ~10 defunct entries
# parented to the live waybar, permanently — they outnumbered everything else
# here. Nothing short of waybar exiting can reap them, so they are noise, not
# signal. Orphaned zombies (reparented to init) still count: those have a parent
# that died owing a wait, which is a real bug in a real program.
stuck() { # space-separated waybar pids; ps state/etimes/ppid/pid/comm on stdin
	awk -v wb="$1" '
	    BEGIN { n = split(wb, a, " "); for (i = 1; i <= n; i++) w[a[i]] = 1 }
	    ($1 == "D" || $1 == "Z") && $2 >= 20 && $3 != 2 && $4 != 2 &&
	    !($1 == "Z" && ($3 in w)) { $3 = ""; print }'
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = "test" ]; then
	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT

	printf 'cpu  100 0 100 700 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\n' > "$T/stat"
	[ "$(snapshot "$T/stat")" = "cpu 900 700
cpu0 500 400" ] || { echo "snapshot wrong: $(snapshot "$T/stat")"; exit 1; }

	# 100 ticks elapsed, 25 of them idle -> 75% busy
	printf 'cpu 900 700\ncpu0 500 400\n' > "$T/prev"
	printf 'cpu 1000 725\ncpu0 600 700\ncpu1 10 5\n' > "$T/cur"
	[ "$(deltas "$T/prev" "$T/cur")" = "cpu 75
cpu0 0" ] || { echo "deltas wrong: $(deltas "$T/prev" "$T/cur")"; exit 1; }

	# a core that went backwards (suspend/resume) must clamp, not go negative
	printf 'cpu 1000 700\n' > "$T/prev"
	printf 'cpu 1000 700\n' > "$T/cur"
	[ "$(deltas "$T/prev" "$T/cur")" = "cpu 0" ] || { echo "zero-delta wrong"; exit 1; }

	# identical cores -> no spread worth printing; mixed ones -> range appended
	printf 'cpu MHz\t\t: 3000.000\ncpu MHz\t\t: 3000.000\n' > "$T/ci"
	[ "$(cpu_freq "$T/ci")" = "3.00 GHz avg" ] || { echo "cpu_freq flat wrong: $(cpu_freq "$T/ci")"; exit 1; }
	printf 'cpu MHz\t\t: 800.000\ncpu MHz\t\t: 4200.000\n' > "$T/ci"
	[ "$(cpu_freq "$T/ci")" = "2.50 GHz avg  ·  0.80–4.20 GHz" ] \
		|| { echo "cpu_freq spread wrong: $(cpu_freq "$T/ci")"; exit 1; }
	: > "$T/ci"
	[ -z "$(cpu_freq "$T/ci")" ] || { echo "cpu_freq should stay silent with no data"; exit 1; }

	# Hybrid: two fast cores then two slow ones -> P0 P1 E2 E3, gap before core 3.
	for c in 0 1 2 3; do
		mkdir -p "$T/sys/cpu$c/cpufreq"
		[ "$c" -lt 2 ] && echo 5000000 > "$T/sys/cpu$c/cpufreq/cpuinfo_max_freq" \
			|| echo 3700000 > "$T/sys/cpu$c/cpufreq/cpuinfo_max_freq"
	done
	CPU_SYS="$T/sys"
	[ "$(core_labels 4)" = "3 P0 P1 E2 E3" ] || { echo "core_labels hybrid wrong: $(core_labels 4)"; exit 1; }

	# Uniform CPU: no split to draw, so no P/E labels and no gap.
	for c in 0 1 2 3; do echo 4000000 > "$T/sys/cpu$c/cpufreq/cpuinfo_max_freq"; done
	[ "$(core_labels 4)" = "0 c0 c1 c2 c3" ] || { echo "core_labels uniform wrong: $(core_labels 4)"; exit 1; }

	# No cpufreq at all (VM, or a kernel without it) must not produce "P" everywhere.
	CPU_SYS="$T/nothing"
	[ "$(core_labels 2)" = "0 c0 c1" ] || { echo "core_labels no-cpufreq wrong: $(core_labels 2)"; exit 1; }
	CPU_SYS="$T/sys"

	# Column-major: 3 cores -> 2 rows, core 0 beside core 2, core 1 alone.
	G=$(coregrid "0 50 100" "P0 P1 E2")
	[ "$(printf '%s\n' "$G" | wc -l)" = 2 ] || { echo "coregrid row count wrong"; exit 1; }
	printf '%s\n' "$G" | head -1 | grep -q 'P0.*░░░░░░░░░░.*  0%.*E2.*██████████.*100%' \
		|| { echo "coregrid layout wrong: $(printf '%s\n' "$G" | head -1)"; exit 1; }
	printf '%s\n' "$G" | tail -1 | grep -q 'P1' || { echo "coregrid dropped the odd core"; exit 1; }
	printf '%s\n' "$G" | tail -1 | grep -q 'E2' && { echo "coregrid repeated a core"; exit 1; }
	# a pinned core must be red even though the row it shares is idle
	printf '%s\n' "$G" | head -1 | grep -q "$C_BAD" || { echo "coregrid should grade per core"; exit 1; }

	# In the 18:27 sample the exited HeapHelper (a whole claude lifetime's CPU) and
	# the Bun Pool thread both outweigh claude and must both lose to it anyway.
	# The post-RESET sample and its since-boot counters must not appear at all.
	printf '%s\n' \
		'RESET' \
		'PRC h 1 2026/08/02 18:17:50 180402 1 (systemd) S 100 999999 999999 0 120 0 0 9 0 1 y 0 () 0 -1 -2 0 0' \
		'SEP' \
		'PRC h 1 2026/08/02 18:27:50 600 2200074 (HeapHelper) E 100 78784 15800 0 0 0 0 -1 0 2200074 y 0 () 0 -2 -2 0 0' \
		'PRC h 1 2026/08/02 18:27:50 600 2820016 (claude) S 100 6000 1800 0 120 0 0 5 0 2820016 y 0 () 0 -2 -2 0 0' \
		'PRC h 1 2026/08/02 18:27:50 600 2820084 (Bun Pool 0) S 100 50000 0 0 120 0 0 5 0 2820016 n 0 () 0 -2 -2 0 0' \
		'PRC h 1 2026/08/02 18:27:50 600 236917 (mango) S 100 2400 0 0 120 0 0 5 0 236917 y 0 () 0 -2 -2 0 0' \
		'SEP' \
		'PRC h 1 2026/08/02 18:37:50 600 2820016 (Bun Pool 3) S 100 30000 0 0 120 0 0 5 0 2820016 y 0 () 0 -2 -2 0 0' \
		'PRC | sys    4m36s | user  16m56s | #proc    425 | #tidle   115 | #exit >52851 |' \
		'SEP' > "$T/prc"
	TAB=$(printf '\t')
	[ "$(atop_top < "$T/prc")" = "18:27${TAB}claude${TAB}13
18:37${TAB}Bun Pool 3${TAB}50" ] || { echo "atop_top wrong: $(atop_top < "$T/prc")"; exit 1; }

	# every row filtered away -> no output at all, rather than a blank row
	[ -z "$(printf '%s\n' 'RESET' 'SEP' \
		'PRC h 1 2026/08/02 18:27:50 600 9 (gone) E 100 100 0 0 0 0 0 -1 0 9 y 0 () 0 -2 -2 0 0' \
		'SEP' | atop_top)" ] || { echo "atop_top should emit nothing when all rows are filtered"; exit 1; }

	# waybar leaves ~10 unreapable zombies behind on every reload; they must not
	# push the genuinely stuck processes out of a four-row section. Everything
	# else about the filter has to survive that: an orphaned zombie still counts,
	# a D-state process counts whoever its parent is, kthreadd and the sub-20s
	# transients stay out.
	[ "$(printf '%s\n' \
		'Z 300 4242 5001 cpu.sh' \
		'Z 300 1 5002 orphan.sh' \
		'D 300 1 5003 blocked' \
		'D 300 2 5004 kworker' \
		'Z 5 4242 5005 quick.sh' \
		'D 300 4242 5006 net.sh' | stuck '4242 4243')" = "Z 300  5002 orphan.sh
D 300  5003 blocked
D 300  5006 net.sh" ] || { echo "stuck wrong: $(printf '%s\n' 'Z 300 4242 5001 cpu.sh' 'Z 300 1 5002 orphan.sh' | stuck '4242')"; exit 1; }

	# No waybar running (or pgrep found nothing) must not swallow every zombie.
	[ "$(printf '%s\n' 'Z 300 4242 5001 cpu.sh' | stuck '')" = "Z 300  5001 cpu.sh" ] \
		|| { echo "stuck with no waybar pids wrong"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- module

NEW="$STATE.new"
snapshot > "$NEW"
if [ ! -f "$STATE" ]; then
	# First run after boot has nothing to diff against — take a short second
	# sample so the module never shows a blank percentage.
	sleep 0.2
	mv "$NEW" "$STATE"
	snapshot > "$NEW"
fi
USAGE=$(deltas "$STATE" "$NEW")
mv "$NEW" "$STATE"

TOTAL=$(printf '%s\n' "$USAGE" | awk '$1 == "cpu" { print $2 }')
[ -n "$TOTAL" ] || TOTAL=0
CORES=$(printf '%s\n' "$USAGE" | awk '$1 != "cpu" { printf "%s ", $2 }')
NCORE=$(printf '%s\n' "$CORES" | wc -w)

CLASS=normal
[ "$TOTAL" -ge 90 ] && CLASS=warning

TEXT="$(barico "$(ic_cpu)") ${TOTAL}%"

TIP=$(
	title "CPU"
	# Fixed width: the rule is the widest line, so this tooltip does not resize
	# as process names come and go. comm is capped at 15 chars by the kernel
	# (TASK_COMM_LEN), so the worst-case row is bounded well under this.
	rule 44

	sect "$IC_LOAD" "Load"
	row "$(printf '%3s%%  %s' "$TOTAL" "$(bar "$TOTAL" "$(grade "$TOTAL" 70 90)")")"
	dim "$(awk -v n="$NCORE" '{ printf "%s / %s / %s  load average  ·  %d threads", $1, $2, $3, n }' /proc/loadavg)"

	sect "$IC_CORES" "Cores"
	# "<gap> <label>..." — split without forking a second time to find the gap
	CL=$(core_labels "$NCORE")
	row "$(heatbar "$CORES" 70 90 "${CL%% *}")  $(printf '%s' "$CORES" | awk '{ mn = 100; mx = 0; for (i = 1; i <= NF; i++) { if ($i < mn) mn = $i; if ($i > mx) mx = $i } printf "%d–%d%%", mn, mx }')"
	printf '\n'
	coregrid "$CORES" "${CL#* }"
	# Pick the package sensor by name; a bare max over every hwmon would report
	# whichever nvme happens to run hottest.
	TEMP=$(for h in /sys/class/hwmon/hwmon*; do
		case "$(cat "$h/name" 2>/dev/null)" in
		coretemp | k10temp | zenpower) cat "$h"/temp1_input 2>/dev/null ;;
		esac
	done | sort -rn | head -1)
	[ -n "$TEMP" ] && TEMP=$(awk -v t="$TEMP" 'BEGIN { printf "%.0f°C", t / 1000 }')
	printf '\n'   # the grid is dense; let the clock/thermal line breathe
	dim "$(printf '%s' "$(cpu_freq)${TEMP:+  ·  $TEMP}")"
	POL=$(cpu_policy)
	[ -n "$POL" ] && dim "$POL"

	sect "$IC_TOP" "Top now"
	# comm goes last: plenty of processes are called "Isolated Web Co", and a
	# trailing field is the only one `read` can hand back with its spaces intact.
	ps -eo pcpu=,pid=,comm= --sort=-pcpu 2>/dev/null | head -5 | while read -r p pid c; do
		row "$(printf '%s  %s <span foreground="%s">%s</span>' \
			"$(mono "$(printf '%s %5s%%' "$(bar "${p%.*}" "$(grade "${p%.*}" 50 80)" 14)" "$p")")" \
			"$(printf '%s' "$c" | esc)" "$C_DIM" "$pid")"
	done

	STUCK=$(ps -eo state=,etimes=,ppid=,pid=,comm= 2> /dev/null |
		stuck "$(pgrep -x waybar 2> /dev/null | tr '\n' ' ')")
	if [ -n "$STUCK" ]; then
		sect "$IC_STUCK" "Stuck"
		printf '%s\n' "$STUCK" | head -4 | while read -r st et pid c; do
			[ "$st" = D ] && what="uninterruptible" || what="zombie"
			row "$(bad "$(printf '%s' "$c" | esc)") <span foreground=\"$C_DIM\">$pid · $what · $(hdur "$et")</span>"
		done
	fi

	# ponytail: today's log only — a tooltip opened at 00:05 shows five minutes
	# of history rather than stitching yesterday's file in.
	LOG="$ATOP_DIR/atop_$(date +%Y%m%d)"
	sect "$IC_HIST" "Recent peaks"
	if [ -r "$LOG" ] && command -v atop > /dev/null 2>&1; then
		atop_stale && atop_refresh "$LOG"
		SAMPLES=$(cat "$ATOP_CACHE" 2> /dev/null || true)
		if [ -n "$SAMPLES" ]; then
			printf '%s\n' "$SAMPLES" | while IFS="$(printf '\t')" read -r tm name pct; do
				[ -n "${pct:-}" ] || continue
				row "$(printf '<span foreground="%s">%s</span>  %s %4s%%' "$C_DIM" "$tm" "$(printf '%s' "$name" | esc)" "$pct")"
			done
		else
			dim "reading today's atop log…"
		fi
	else
		dim "atop history unavailable"
	fi
)

emit "$CLASS" "$TEXT" "$TIP"
