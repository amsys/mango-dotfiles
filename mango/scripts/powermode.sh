#!/bin/sh
# Full-performance / eco power modes, driven by the AC cable and overridable
# by a click. Everything here is userspace; the handful of root-owned knobs
# (CPU EPP, turbo, PCI/NVMe runtime PM, ...) are delegated to
# system/powermode/mango-powermode, which re-validates every value itself —
# this script is not the trust boundary, that one is.
#
# State lives in $XDG_RUNTIME_DIR so it resets on reboot rather than
# outliving a config change:
#   mango-powermode           current mode: eco|full
#   mango-powermode.manual    present -> a click overrode the cable
#   mango-powermode.bright    backlight %, remembered on eco entry
#   mango-powermode.weak      weak-charger latch, set by battery-guard.sh
#   mango-powermode.drain     pid of the running drain loop (see below)
#   mango-powermode.ollama    model names unloaded on eco entry
#
#   powermode.sh auto      recompute mode from AC + markers (no-op if manual)
#   powermode.sh cable     AC plug/unplug edge — clear markers, then auto
#   powermode.sh toggle    flip mode, set the manual marker   (battery pill click)
#   powermode.sh eco|full  force a mode directly, set the manual marker
#   powermode.sh weak      force eco, latch weak (idempotent)
#   powermode.sh unweak    clear the weak latch, then auto     (recovery)
#   powermode.sh drain     internal: the eco wait/pause loop, see set_mode()
#   powermode.sh status    print the current mode + markers
#   powermode.sh test      assert the decision table, no hardware touched
#
# Eco entry pauses hermes right away, then waits for any open omp/pi
# coding-agent session to go idle before touching docker/paseo/ollama — see
# drain() below. Full entry cancels a wait in flight and undoes all of it.
set -u

RUN="${XDG_RUNTIME_DIR:-/tmp}"
MODE_FILE="$RUN/mango-powermode"
MANUAL_FILE="$RUN/mango-powermode.manual"
BRIGHT_FILE="$RUN/mango-powermode.bright"
WEAK_FILE="$RUN/mango-powermode.weak"
BARS_PID="$RUN/mango-bars.pid"
DRAIN_PID="$RUN/mango-powermode.drain"
OLLAMA_FILE="$RUN/mango-powermode.ollama"

CONF="${MANGO_POWERMODE_CONF:-$HOME/.config/mango/powermode.conf}"
AC="${MANGO_AC_DIR:-}"

# powermode.conf is the user's own file, sourced the same way mango sources
# local.conf — this is userspace reading userspace, not the root boundary.
# shellcheck disable=SC1090  # user-owned config file
[ -r "$CONF" ] && . "$CONF"

PM_FULL_EPP=${PM_FULL_EPP:-performance}
PM_ECO_EPP=${PM_ECO_EPP:-power}
PM_FULL_PROFILE=${PM_FULL_PROFILE:-performance}
PM_ECO_PROFILE=${PM_ECO_PROFILE:-quiet}
PM_FULL_NO_TURBO=${PM_FULL_NO_TURBO:-0}
PM_ECO_NO_TURBO=${PM_ECO_NO_TURBO:-1}
PM_FULL_PCI_PM=${PM_FULL_PCI_PM:-baseline}
PM_ECO_PCI_PM=${PM_ECO_PCI_PM:-auto}
PM_FULL_SND_HDA_POWERSAVE=${PM_FULL_SND_HDA_POWERSAVE:-baseline}
PM_ECO_SND_HDA_POWERSAVE=${PM_ECO_SND_HDA_POWERSAVE:-1}
PM_FULL_VM_LAPTOP_MODE=${PM_FULL_VM_LAPTOP_MODE:-0}
PM_ECO_VM_LAPTOP_MODE=${PM_ECO_VM_LAPTOP_MODE:-5}
PM_FULL_VM_DIRTY_WB=${PM_FULL_VM_DIRTY_WB:-500}
PM_ECO_VM_DIRTY_WB=${PM_ECO_VM_DIRTY_WB:-1500}
PM_FULL_WIFI_POWERSAVE=${PM_FULL_WIFI_POWERSAVE:-off}
PM_ECO_WIFI_POWERSAVE=${PM_ECO_WIFI_POWERSAVE:-on}
PM_ECO_BRIGHT=${PM_ECO_BRIGHT-40}
PM_ECO_DOCKER_PREFIX=${PM_ECO_DOCKER_PREFIX-frappe-}
PM_ECO_BUSY_PROCS=${PM_ECO_BUSY_PROCS-claude}
PM_ECO_HERMES_MATCH=${PM_ECO_HERMES_MATCH-hermes-agent/hermes}
PM_ECO_AGENT_MATCH=${PM_ECO_AGENT_MATCH-pi-coding-agent}
PM_ECO_DRAIN_POLL=${PM_ECO_DRAIN_POLL:-30}
PM_ECO_DRAIN_IDLE_CPU=${PM_ECO_DRAIN_IDLE_CPU:-1}
PM_ECO_DRAIN_FORCE_PCT=${PM_ECO_DRAIN_FORCE_PCT:-40}
PM_ECO_PASEO_ROOM=${PM_ECO_PASEO_ROOM-power}
PM_ECO_OLLAMA_RESTORE=${PM_ECO_OLLAMA_RESTORE:-0}

# ---------------------------------------------------------------- primitives

current_mode() { [ -r "$MODE_FILE" ] && cat "$MODE_FILE" 2> /dev/null || printf full; }

payload() { # mode -> KEY=value lines for mango-powermode apply
	if [ "$1" = eco ]; then
		cat << EOF
MODE=eco
EPP=$PM_ECO_EPP
PROFILE=$PM_ECO_PROFILE
NO_TURBO=$PM_ECO_NO_TURBO
PCI_PM=$PM_ECO_PCI_PM
SND_HDA_POWERSAVE=$PM_ECO_SND_HDA_POWERSAVE
VM_LAPTOP_MODE=$PM_ECO_VM_LAPTOP_MODE
VM_DIRTY_WB=$PM_ECO_VM_DIRTY_WB
WIFI_POWERSAVE=$PM_ECO_WIFI_POWERSAVE
EOF
	else
		cat << EOF
MODE=full
EPP=$PM_FULL_EPP
PROFILE=$PM_FULL_PROFILE
NO_TURBO=$PM_FULL_NO_TURBO
PCI_PM=$PM_FULL_PCI_PM
SND_HDA_POWERSAVE=$PM_FULL_SND_HDA_POWERSAVE
VM_LAPTOP_MODE=$PM_FULL_VM_LAPTOP_MODE
VM_DIRTY_WB=$PM_FULL_VM_DIRTY_WB
WIFI_POWERSAVE=$PM_FULL_WIFI_POWERSAVE
EOF
	fi
}

# Every real side effect funnels through these four, so `test` can stub them
# all out with one flag instead of faking sudo, brightnessctl and docker.
apply_root() { [ -n "${MANGO_PM_TEST:-}" ] || payload "$1" | sudo -n /usr/local/bin/mango-powermode apply > /dev/null 2>&1; }

enter_eco_bright() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -n "$PM_ECO_BRIGHT" ] || return 0
	command -v brightnessctl > /dev/null 2>&1 || return 0
	cur=$(brightnessctl -m 2> /dev/null | awk -F, '{ gsub("%", "", $4); print $4 }')
	[ -n "$cur" ] && printf '%s\n' "$cur" > "$BRIGHT_FILE"
	brightnessctl set "${PM_ECO_BRIGHT}%" > /dev/null 2>&1
}
exit_eco_bright() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -f "$BRIGHT_FILE" ] || return 0
	prev=$(cat "$BRIGHT_FILE" 2> /dev/null)
	rm -f "$BRIGHT_FILE"
	[ -n "$prev" ] && command -v brightnessctl > /dev/null 2>&1 && brightnessctl set "${prev}%" > /dev/null 2>&1
}

# Stopping containers unconditionally kills whatever's running in them. Three
# outcomes instead of one:
#   a docker exec/build is attached  -> leave running (stop kills it, pause
#                                        hangs the client)
#   an agent session is open, docker idle -> pause (freezes the CPU burners,
#                                        resumes instantly, and docker exec
#                                        against a paused container errors
#                                        cleanly instead of hanging)
#   neither                          -> stop (today's behaviour)
# Pure filters take `ps` output on stdin so the self-check needs no processes,
# same split waybar/scripts/docker.sh uses for its own canned-input test.

# comm+args lines (ps -eo comm=,args=) -> exit 0 if a PM_ECO_BUSY_PROCS name is
# running something other than claude's own daemon/bg-pty-host/bg-spare
# infrastructure, which exists whether or not a session is open.
agent_busy_filter() {
	[ -n "$PM_ECO_BUSY_PROCS" ] || return 1
	awk -v procs="$PM_ECO_BUSY_PROCS" '
		BEGIN { n = split(procs, want, " "); for (i = 1; i <= n; i++) wantset[want[i]] = 1 }
		!($1 in wantset) { next }
		/daemon run/ || /bg-pty-host/ || /bg-spare/ { next }
		{ found = 1 }
		END { exit !found }
	'
}
agent_busy() { ps -eo comm=,args= 2> /dev/null | agent_busy_filter; }

# args lines (ps -eo args=) -> exit 0 if a docker/docker-compose client is
# running exec/build/run/attach/cp. Deliberately excludes `ps` (waybar polls it
# every 10s) and stop/pause/unpause (our own calls a moment later).
docker_busy_filter() {
	awk '
		$1 ~ /(^|\/)docker(-compose)?$/ {
			for (i = 2; i <= NF; i++) if ($i ~ /^(exec|build|run|attach|cp)$/) { found = 1; break }
		}
		END { exit !found }
	'
}
docker_busy() { ps -eo args= 2> /dev/null | docker_busy_filter; }

docker_action() { # agent-busy(0|1) docker-busy(0|1) -> run|pause|stop
	if [ "$2" = 0 ]; then printf run
	elif [ "$1" = 0 ]; then printf pause
	else printf stop
	fi
}

notify_docker() { # action(run|pause|stop), reason
	[ "$1" = stop ] && return 0
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	command -v notify-send > /dev/null 2>&1 || return 0
	title="🐳 containers left running"
	[ "$1" = pause ] && title="🐳 containers paused"
	notify-send -a powermode -u low \
		-h string:x-canonical-private-synchronous:powermode-docker \
		"$title" "$2"
}

# Detached: eight containers at a 10s SIGTERM grace apiece would otherwise
# stall whatever called this (an unplug, a click). Pause is instant but stays
# in the same subshell for one code path.
docker_eco() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -n "$PM_ECO_DOCKER_PREFIX" ] || return 0
	command -v docker > /dev/null 2>&1 || return 0
	names=$(docker ps --format '{{.Names}}' 2> /dev/null | grep "^$PM_ECO_DOCKER_PREFIX")
	[ -n "$names" ] || return 0

	a=1
	agent_busy && a=0
	d=1
	docker_busy && d=0
	action=$(docker_action "$a" "$d")

	case "$action" in
	pause)
		reason="an agent session is running"
		(printf '%s\n' "$names" | xargs -r docker pause > /dev/null 2>&1 &)
		;;
	stop)
		reason=""
		(printf '%s\n' "$names" | xargs -r docker stop > /dev/null 2>&1 &)
		;;
	*) reason="a docker command is in flight" ;;
	esac
	notify_docker "$action" "$reason"
}

# Containers stopped in eco stay stopped on the way back to full, by design —
# start a bench on demand with `frappe-dev <env> up`. Paused ones didn't ask
# to be stopped, so they come straight back.
docker_unpause() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -n "$PM_ECO_DOCKER_PREFIX" ] || return 0
	command -v docker > /dev/null 2>&1 || return 0
	names=$(docker ps --filter status=paused --format '{{.Names}}' 2> /dev/null | grep "^$PM_ECO_DOCKER_PREFIX")
	[ -n "$names" ] || return 0
	printf '%s\n' "$names" | xargs -r docker unpause > /dev/null 2>&1
}

# TSTP not STOP: catchable, so a hermes that traps it for its own graceful
# pause gets the chance to; if it doesn't trap it the default action is the
# same freeze either way.
hermes_pause() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -n "$PM_ECO_HERMES_MATCH" ] || return 0
	pkill -TSTP -f "$PM_ECO_HERMES_MATCH" 2> /dev/null
}
hermes_resume() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -n "$PM_ECO_HERMES_MATCH" ] || return 0
	pkill -CONT -f "$PM_ECO_HERMES_MATCH" 2> /dev/null
}

# omp and pi both run under an interpreter (bun, node) shared with unrelated
# dev tools, so `ps -eo comm=` (bun, node-MainThread) can't tell them apart —
# matching has to be on the full command line, same reasoning as
# PM_ECO_HERMES_MATCH above.
#
# /proc/<pid>/stat's comm field (2nd, in parens) can itself contain spaces or
# parens, so utime/stime are found by scanning from the end for the LAST ')'
# rather than assuming a fixed field number — the kernel's own advice for
# parsing this file.
agent_ticks() { # -> "ticks nprocs", combined utime+stime across matches
	[ -n "$PM_ECO_AGENT_MATCH" ] || { printf '0 0'; return 0; }
	total=0
	n=0
	for pid in $(pgrep -f "$PM_ECO_AGENT_MATCH" 2> /dev/null); do
		stat="/proc/$pid/stat"
		[ -r "$stat" ] || continue
		ticks=$(awk '{
			for (i = NF; i > 0; i--) if ($i ~ /\)$/) { p = i; break }
			print $(p + 12) + $(p + 13)
		}' "$stat" 2> /dev/null)
		case "$ticks" in '' | *[!0-9]*) continue ;; esac
		total=$((total + ticks))
		n=$((n + 1))
	done
	printf '%s %s' "$total" "$n"
}

# Pure: nprocs, CPU ticks used since the last sample, the idle threshold in
# the same units, current battery %, and the force-drain floor -> drain|wait.
# No processes and a low battery both win outright; only "quiet enough since
# last sample" needs the caller to have actually waited one.
drain_decision() { # nprocs delta_ticks idle_ticks pct force_pct -> drain|wait
	n=$1 delta=$2 idle=$3 pct=$4 force=$5
	if [ "$n" -eq 0 ]; then printf drain
	elif [ "$pct" -lt "$force" ]; then printf drain
	elif [ "$delta" -lt "$idle" ]; then printf drain
	else printf wait
	fi
}

battery_pct() {
	bat="${MANGO_BAT_DIR:-}"
	[ -n "$bat" ] || bat=$(set -- /sys/class/power_supply/BAT*; printf '%s' "$1")
	[ -r "$bat/capacity" ] && cat "$bat/capacity" 2> /dev/null || printf 100
}

paseo_notify() { # message
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -n "$PM_ECO_PASEO_ROOM" ] || return 0
	command -v paseo > /dev/null 2>&1 || return 0
	paseo chat post "$PM_ECO_PASEO_ROOM" "$1" > /dev/null 2>&1 && return 0
	paseo chat create "$PM_ECO_PASEO_ROOM" > /dev/null 2>&1
	paseo chat post "$PM_ECO_PASEO_ROOM" "$1" > /dev/null 2>&1
}

# `ollama stop` unloads a model from RAM/VRAM without touching ollama.service
# itself — the daemon stays up, just idle. Names are recorded so a later
# ollama_restore (opt-in, see PM_ECO_OLLAMA_RESTORE) knows what to re-warm.
ollama_unload() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	command -v ollama > /dev/null 2>&1 || return 0
	names=$(ollama ps 2> /dev/null | awk 'NR > 1 { print $1 }')
	[ -n "$names" ] || return 0
	printf '%s\n' "$names" > "$OLLAMA_FILE"
	printf '%s\n' "$names" | while IFS= read -r m; do ollama stop "$m" > /dev/null 2>&1; done
}

ollama_restore() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -f "$OLLAMA_FILE" ] || return 0
	names=$(cat "$OLLAMA_FILE" 2> /dev/null)
	rm -f "$OLLAMA_FILE"
	[ "$PM_ECO_OLLAMA_RESTORE" = 1 ] || return 0
	command -v ollama > /dev/null 2>&1 || return 0
	printf '%s\n' "$names" | while IFS= read -r m; do
		[ -n "$m" ] && (ollama run "$m" hi < /dev/null > /dev/null 2>&1 &)
	done
}

# Verified against /proc/<pid>/comm, not just "a pid is in the file" — same
# stale-pid guard as bars_restart below, for the same reason: a recycled pid
# must never be signalled.
drain_stop() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -r "$DRAIN_PID" ] || return 0
	pid=$(cat "$DRAIN_PID" 2> /dev/null)
	rm -f "$DRAIN_PID"
	[ -n "$pid" ] || return 0
	[ "$(cat "/proc/$pid/comm" 2> /dev/null)" = powermode.sh ] || return 0
	kill "$pid" 2> /dev/null
}

# Detached: the caller (set_mode, from a short-lived ac-watch.sh/toggle
# invocation) must not block on however long omp/pi keep working.
drain_start() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	drain_stop
	(setsid "$0" drain > /dev/null 2>&1 &)
}

# The eco wait/pause loop itself — see the header comment for the sequence.
# Runs as its own detached process (drain_start), so every exit path must
# clean up $DRAIN_PID itself; nothing else will.
drain() {
	printf '%s' "$$" > "$DRAIN_PID"
	trap 'rm -f "$DRAIN_PID"' EXIT

	idle_ticks=$((PM_ECO_DRAIN_IDLE_CPU * 100))
	# shellcheck disable=SC2046  # deliberate: splitting agent_ticks' "ticks nprocs" into $1 $2
	set -- $(agent_ticks)
	prev_ticks=$1
	n=$2

	# First check passes delta=idle_ticks on purpose: no poll interval has
	# elapsed yet, so only "nobody running" and "battery already low" can
	# end the wait before one has — "did they go idle" needs a real
	# interval, which the loop below measures.
	verdict=$(drain_decision "$n" "$idle_ticks" "$idle_ticks" "$(battery_pct)" "$PM_ECO_DRAIN_FORCE_PCT")
	while [ "$verdict" = wait ]; do
		sleep "$PM_ECO_DRAIN_POLL"
		[ "$(current_mode)" = eco ] || exit 0
		# shellcheck disable=SC2046  # deliberate: splitting agent_ticks' "ticks nprocs" into $1 $2
		set -- $(agent_ticks)
		delta=$(($1 - prev_ticks))
		prev_ticks=$1
		n=$2
		verdict=$(drain_decision "$n" "$delta" "$idle_ticks" "$(battery_pct)" "$PM_ECO_DRAIN_FORCE_PCT")
	done

	[ "$(current_mode)" = eco ] || exit 0
	docker_eco
	paseo_notify "🔋 on battery — containers paused, ollama unloaded"
	ollama_unload
}

# USR1, not HUP: bars.sh traps USR1 rather than HUP because a HUP-preignoring
# parent (nohup among others) makes HUP permanently untrappable in bash — see
# the matching comment in bars.sh.
bars_restart() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -r "$BARS_PID" ] || return 0
	pid=$(cat "$BARS_PID" 2> /dev/null)
	[ -n "$pid" ] || return 0
	# bars.sh's degraded-launch path used to be able to leave this file naming
	# waybar itself rather than bars.sh — guarded there now too, but cheap
	# insurance here: waybar's own SIGUSR1 default is "toggle visibility", not
	# reload, so signalling it by mistake hides the bar until the next real
	# toggle rather than just delaying one.
	[ "$(cat "/proc/$pid/comm" 2> /dev/null)" = bars.sh ] || return 0
	kill -USR1 "$pid" 2> /dev/null
}

notify_mode() { # mode, detail
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	command -v notify-send > /dev/null 2>&1 || return 0
	icon=$([ "$1" = eco ] && printf '🌿' || printf '⚡')
	notify-send -a powermode -u low \
		-h string:x-canonical-private-synchronous:powermode \
		"$icon $1 mode" "${2:-}"
}

# ---------------------------------------------------------------- mode set

set_mode() { # mode
	m=$1
	printf '%s' "$m" > "$MODE_FILE"
	apply_root "$m"
	if [ "$m" = eco ]; then
		enter_eco_bright
		hermes_pause
		drain_start
	else
		exit_eco_bright
		drain_stop
		hermes_resume
		docker_unpause
		ollama_restore
	fi
	bars_restart
	[ -n "${MANGO_PM_TEST:-}" ] || pkill -RTMIN+11 waybar 2> /dev/null
}

online() {
	[ -n "$AC" ] || AC=$(set -- /sys/class/power_supply/A[CD]*; printf '%s' "$1")
	[ -r "$AC/online" ] && cat "$AC/online" 2> /dev/null || printf 0
}

auto() {
	if [ -f "$WEAK_FILE" ]; then set_mode eco; return; fi
	[ -f "$MANUAL_FILE" ] && return # a click already decided; the cable doesn't override it
	if [ "$(online)" = 1 ]; then set_mode full; else set_mode eco; fi
}

cable() { # AC plug/unplug edge: a new cable state is a new decision
	rm -f "$MANUAL_FILE" "$WEAK_FILE"
	auto
}

toggle() {
	new=eco
	[ "$(current_mode)" = eco ] && new=full
	: > "$MANUAL_FILE"
	set_mode "$new"
	notify_mode "$new" manual
}

force() { # mode
	: > "$MANUAL_FILE"
	set_mode "$1"
	notify_mode "$1" manual
}

weak() {
	: > "$WEAK_FILE"
	rm -f "$MANUAL_FILE"
	set_mode eco
}

unweak() {
	rm -f "$WEAK_FILE"
	auto
}

status() {
	printf 'mode:   %s\n' "$(current_mode)"
	printf 'manual: %s\n' "$([ -f "$MANUAL_FILE" ] && echo yes || echo no)"
	printf 'weak:   %s\n' "$([ -f "$WEAK_FILE" ] && echo yes || echo no)"
	printf 'AC:     %s\n' "$([ "$(online)" = 1 ] && echo online || echo offline)"
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	MANGO_PM_TEST=1
	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT
	RUN="$T"
	MODE_FILE="$RUN/mode" MANUAL_FILE="$RUN/manual" BRIGHT_FILE="$RUN/bright" WEAK_FILE="$RUN/weak" BARS_PID="$RUN/bars.pid"
	DRAIN_PID="$RUN/drain.pid" OLLAMA_FILE="$RUN/ollama"

	mkdir -p "$T/ac"
	AC="$T/ac"
	echo 1 > "$T/ac/online"

	# AC online, nothing forced -> full
	auto
	[ "$(current_mode)" = full ] || { echo "auto on AC should pick full: $(current_mode)"; exit 1; }

	# unplug -> eco
	echo 0 > "$T/ac/online"
	auto
	[ "$(current_mode)" = eco ] || { echo "auto off AC should pick eco: $(current_mode)"; exit 1; }

	# a manual override survives a repeated auto() with the cable unchanged
	toggle # eco -> full, sets manual
	[ "$(current_mode)" = full ] || { echo "toggle should have flipped to full"; exit 1; }
	[ -f "$MANUAL_FILE" ] || { echo "toggle should set the manual marker"; exit 1; }
	auto # still on battery (online=0), but manual should win
	[ "$(current_mode)" = full ] || { echo "auto must not override a manual choice"; exit 1; }

	# a cable edge clears manual and re-decides
	cable
	[ "$(current_mode)" = eco ] || { echo "cable edge should clear manual and re-decide from AC state: $(current_mode)"; exit 1; }
	[ -f "$MANUAL_FILE" ] && { echo "cable edge should have cleared the manual marker"; exit 1; }

	# weak charger forces eco even on AC, and outranks a manual full
	echo 1 > "$T/ac/online"
	force full
	[ "$(current_mode)" = full ] || { echo "force full failed"; exit 1; }
	weak
	[ "$(current_mode)" = eco ] || { echo "weak should force eco even while on AC: $(current_mode)"; exit 1; }
	[ -f "$WEAK_FILE" ] || { echo "weak should set the latch"; exit 1; }
	auto # latch must keep winning until unweak clears it
	[ "$(current_mode)" = eco ] || { echo "auto must keep eco while the weak latch is set"; exit 1; }

	# recovery: unweak clears the latch and re-decides — AC is online, so full
	unweak
	[ -f "$WEAK_FILE" ] && { echo "unweak should clear the latch"; exit 1; }
	[ "$(current_mode)" = full ] || { echo "unweak should re-decide from AC state: $(current_mode)"; exit 1; }

	# payload shape: every key present, mode-appropriate values
	P=$(payload eco)
	printf '%s\n' "$P" | grep -qx 'MODE=eco' || { echo "eco payload missing MODE"; exit 1; }
	printf '%s\n' "$P" | grep -qx "EPP=$PM_ECO_EPP" || { echo "eco payload missing EPP"; exit 1; }
	P=$(payload full)
	printf '%s\n' "$P" | grep -qx 'MODE=full' || { echo "full payload missing MODE"; exit 1; }

	# agent_busy_filter: claude's own daemon/bg-pty-host/bg-spare infra must not
	# read as a session, a real --session-id process must
	AGENT_IDLE='claude claude daemon run --origin transient --spawned-by {"label":"claude","cwd":"/home/martin/src/crema","pid":3201856}
claude claude bg-pty-host --bg-pty-host /tmp/cc-daemon-1000/x/spare/0c84e39e.pty.sock 200 50 -- /opt/claude-code/bin/claude --bg-spare /tmp/cc-daemon-1000/x/spare/0c84e39e.claim.sock
claude claude bg-spare --bg-spare /tmp/cc-daemon-1000/x/spare/0c84e39e.claim.sock'
	PM_ECO_BUSY_PROCS=claude
	printf '%s\n' "$AGENT_IDLE" | agent_busy_filter && { echo "agent_busy_filter should ignore claude infrastructure"; exit 1; }

	AGENT_BUSY="$AGENT_IDLE
claude /opt/claude-code/bin/claude --session-id 21afdf13-ef95-49e7-9f05-9c8761d398ef --fork-session --resume /home/martin/.claude/projects/x.jsonl --permission-mode auto"
	printf '%s\n' "$AGENT_BUSY" | agent_busy_filter || { echo "agent_busy_filter should catch a real claude session"; exit 1; }

	PM_ECO_BUSY_PROCS=
	printf '%s\n' "$AGENT_BUSY" | agent_busy_filter && { echo "empty PM_ECO_BUSY_PROCS should disable agent detection"; exit 1; }
	PM_ECO_BUSY_PROCS=claude

	# docker_busy_filter: dockerd/docker-proxy/`docker ps`/our own stop must not
	# read as busy, an attached exec must
	DOCKER_IDLE='/usr/bin/dockerd -H fd:// --containerd=/run/containerd/containerd.sock
/usr/bin/docker-proxy -proto tcp -host-ip 0.0.0.0 -host-port 8017 -container-ip 172.20.0.5 -container-port 8000 -use-listen-fd
docker ps --format {{.Names}}|{{.State}}
docker stop frappe-version-16-bench-1'
	printf '%s\n' "$DOCKER_IDLE" | docker_busy_filter && { echo "docker_busy_filter should ignore daemon/proxy/ps/stop"; exit 1; }

	DOCKER_BUSY="$DOCKER_IDLE
docker compose -p frappe-version-16 -f /home/martin/src/workbench/tools/docker-frappe/compose.yml exec -T bench bash -c cd /home/frappe/frappe-bench"
	printf '%s\n' "$DOCKER_BUSY" | docker_busy_filter || { echo "docker_busy_filter should catch a live exec"; exit 1; }

	# docker_action: docker in flight wins (leave running) regardless of agent;
	# agent alone pauses; neither stops
	[ "$(docker_action 1 1)" = stop ] || { echo "docker_action idle,idle should be stop"; exit 1; }
	[ "$(docker_action 0 1)" = pause ] || { echo "docker_action agent-busy,docker-idle should be pause"; exit 1; }
	[ "$(docker_action 1 0)" = run ] || { echo "docker_action agent-idle,docker-busy should be run"; exit 1; }
	[ "$(docker_action 0 0)" = run ] || { echo "docker_action both-busy should be run (docker wins)"; exit 1; }

	# drain_decision: nprocs, delta_ticks, idle_ticks, pct, force_pct -> drain|wait
	[ "$(drain_decision 0 999 100 80 40)" = drain ] || { echo "drain_decision should drain with no processes"; exit 1; }
	[ "$(drain_decision 2 50 100 80 40)" = drain ] || { echo "drain_decision should drain when CPU delta is under idle threshold"; exit 1; }
	[ "$(drain_decision 2 999 100 80 40)" = wait ] || { echo "drain_decision should wait when busy and battery is healthy"; exit 1; }
	[ "$(drain_decision 2 999 100 20 40)" = drain ] || { echo "drain_decision should drain when battery is below force_pct, regardless of activity"; exit 1; }
	[ "$(drain_decision 2 999 100 40 40)" = wait ] || { echo "drain_decision at exactly force_pct should still wait, not drain"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- dispatch

case "${1:-}" in
auto) auto ;;
cable) cable ;;
toggle) toggle ;;
eco) force eco ;;
full) force full ;;
weak) weak ;;
unweak) unweak ;;
drain) drain ;;
status) status ;;
*)
	echo "usage: powermode.sh auto|cable|toggle|eco|full|weak|unweak|drain|status|test" >&2
	exit 1
	;;
esac
