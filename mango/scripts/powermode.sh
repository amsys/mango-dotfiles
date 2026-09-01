#!/bin/sh
# Full-performance / eco power modes, driven by the AC cable and overridable
# by a click. Everything here is userspace; the handful of root-owned knobs
# (CPU EPP, turbo, PCI/NVMe runtime PM, ...) are delegated to
# system/powermode/mango-powermode, which re-validates every value itself —
# this script is not the trust boundary, that one is.
#
# Three modes: full (AC), battery (unplugged, work continues), eco (battery
# low, weak charger, or a click — free RAM, freeze the busy work). battery is
# a mild hardware-only saving; only eco pauses/stops/unloads anything.
#
# State lives in $XDG_RUNTIME_DIR so it resets on reboot rather than
# outliving a config change:
#   mango-powermode           current mode: eco|full|battery
#   mango-powermode.manual    present -> a click or a low-battery escalation
#                              overrode the cable
#   mango-powermode.bright    backlight %, remembered on first dim (battery or eco)
#   mango-powermode.weak      weak-charger latch, set by battery-guard.sh
#   mango-powermode.drain     pid of the running drain loop (see below)
#   mango-powermode.ollama    model names unloaded on eco entry
#   mango-powermode.charge    charge-limit override (e.g. before travel);
#                              absent -> PM_CHARGE_LIMIT from powermode.conf
#   mango-powermode.lock      flock target serializing every mode-changing
#                              call below — ac-watch.sh backgrounds a `cable`
#                              per udev edge, so a bouncing USB-C jack can
#                              fire several before the first returns; without
#                              this their set_mode()/enter_dim() writes
#                              interleave and the machine can end up
#                              throttled+dimmed while plugged in, or restore
#                              the wrong pre-dim brightness
#
#   powermode.sh auto      recompute mode from AC + battery % (no-op if manual)
#   powermode.sh cable     AC plug/unplug edge — clear markers, then auto
#   powermode.sh toggle    flip full<->eco, set the manual marker (battery pill click)
#   powermode.sh eco|full|battery  force a mode directly, set the manual marker
#   powermode.sh low       force eco, e.g. battery fell under PM_BAT_ECO_PCT
#   powermode.sh weak      force eco, latch weak (idempotent)
#   powermode.sh unweak    clear the weak latch, then auto     (recovery)
#   powermode.sh charge N  set the charge ceiling override, then reapply
#                          (N == PM_CHARGE_LIMIT clears the override instead)
#   powermode.sh drain     internal: the eco wait/pause loop, see set_mode()
#   powermode.sh status    print the current mode + markers
#   powermode.sh test      assert the decision table, no hardware touched
#
# The charge ceiling (mango-powermode.charge, PM_CHARGE_LIMIT) is orthogonal
# to full/battery/eco: cable() and weak()/unweak() never touch it, so
# unplugging or a weak-charger latch never discards a travel override.
#
# Eco entry pauses hermes right away, then waits for any open omp/pi
# coding-agent session to go idle before touching docker/paseo/ollama — see
# drain() below. Full or battery entry cancels a wait in flight and undoes
# all of it.
set -u

RUN="${XDG_RUNTIME_DIR:-/tmp}"
MODE_FILE="$RUN/mango-powermode"
MANUAL_FILE="$RUN/mango-powermode.manual"
BRIGHT_FILE="$RUN/mango-powermode.bright"
WEAK_FILE="$RUN/mango-powermode.weak"
DRAIN_PID="$RUN/mango-powermode.drain"
OLLAMA_FILE="$RUN/mango-powermode.ollama"
CHARGE_FILE="$RUN/mango-powermode.charge"
LOCK_FILE="$RUN/mango-powermode.lock"

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
# battery keeps the CPU responsive (default EPP/turbo/profile) and only takes
# eco's I/O-side savings (PCI/NVMe PM, snd_hda, vm.laptop_mode, wifi) — those
# cost nothing in responsiveness, so there is no separate PM_BAT_PCI_PM etc.
PM_BAT_EPP=${PM_BAT_EPP:-balance_power}
PM_BAT_PROFILE=${PM_BAT_PROFILE:-balanced}
PM_BAT_NO_TURBO=${PM_BAT_NO_TURBO:-0}
PM_BAT_BRIGHT=${PM_BAT_BRIGHT-70}
PM_BAT_ECO_PCT=${PM_BAT_ECO_PCT:-40}
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
PM_CHARGE_LIMIT=${PM_CHARGE_LIMIT:-80}

# ---------------------------------------------------------------- primitives

current_mode() { [ -r "$MODE_FILE" ] && cat "$MODE_FILE" 2> /dev/null || printf full; }

# The travel override in $CHARGE_FILE, if any, else PM_CHARGE_LIMIT.
charge_limit() { [ -r "$CHARGE_FILE" ] && cat "$CHARGE_FILE" 2> /dev/null || printf '%s' "$PM_CHARGE_LIMIT"; }

payload() { # mode -> KEY=value lines for mango-powermode apply
	case "$1" in
	full)
		epp=$PM_FULL_EPP prof=$PM_FULL_PROFILE turbo=$PM_FULL_NO_TURBO
		pci=$PM_FULL_PCI_PM snd=$PM_FULL_SND_HDA_POWERSAVE
		lm=$PM_FULL_VM_LAPTOP_MODE wb=$PM_FULL_VM_DIRTY_WB wifi=$PM_FULL_WIFI_POWERSAVE
		;;
	battery)
		epp=$PM_BAT_EPP prof=$PM_BAT_PROFILE turbo=$PM_BAT_NO_TURBO
		pci=$PM_ECO_PCI_PM snd=$PM_ECO_SND_HDA_POWERSAVE
		lm=$PM_ECO_VM_LAPTOP_MODE wb=$PM_ECO_VM_DIRTY_WB wifi=$PM_ECO_WIFI_POWERSAVE
		;;
	*)
		epp=$PM_ECO_EPP prof=$PM_ECO_PROFILE turbo=$PM_ECO_NO_TURBO
		pci=$PM_ECO_PCI_PM snd=$PM_ECO_SND_HDA_POWERSAVE
		lm=$PM_ECO_VM_LAPTOP_MODE wb=$PM_ECO_VM_DIRTY_WB wifi=$PM_ECO_WIFI_POWERSAVE
		;;
	esac
	cat << EOF
MODE=$1
EPP=$epp
PROFILE=$prof
NO_TURBO=$turbo
PCI_PM=$pci
SND_HDA_POWERSAVE=$snd
VM_LAPTOP_MODE=$lm
VM_DIRTY_WB=$wb
WIFI_POWERSAVE=$wifi
CHARGE_LIMIT=$(charge_limit)
EOF
}

# Every real side effect funnels through these four, so `test` can stub them
# all out with one flag instead of faking sudo, brightnessctl and docker.
apply_root() { [ -n "${MANGO_PM_TEST:-}" ] || payload "$1" | sudo -n /usr/local/bin/mango-powermode apply > /dev/null 2>&1; }

# First-touch-wins on $BRIGHT_FILE: full -> battery(70%) -> eco(40%) must
# restore the *pre-battery* level, not overwrite it with 70 on the way down —
# same idiom as mango-powermode's baseline_put(), same reason.
enter_dim() { # target-percent
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	[ -n "$1" ] || return 0
	command -v brightnessctl > /dev/null 2>&1 || return 0
	if [ ! -f "$BRIGHT_FILE" ]; then
		cur=$(brightnessctl -m 2> /dev/null | awk -F, '{ gsub("%", "", $4); print $4 }')
		[ -n "$cur" ] && printf '%s\n' "$cur" > "$BRIGHT_FILE"
	fi
	brightnessctl set "${1}%" > /dev/null 2>&1
}
exit_dim() {
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

# `pkill -f` matches the whole argv as a substring, so the bare pattern used
# to also catch anything ELSE whose argv happened to contain it — a
# `vim ~/src/hermes-agent/hermes.py`, a `tail -f` on its log, a grep typed by
# hand — and freeze it until AC returned. hermes runs under its venv's own
# python (powermode.conf's own comment), so filtering matches down to a
# python interpreter's /proc/<pid>/exe rejects those three real examples
# (vim, tail, grep — none of them python) while still catching the genuine
# target.
hermes_pids() {
	[ -n "$PM_ECO_HERMES_MATCH" ] || return 0
	for pid in $(pgrep -f "$PM_ECO_HERMES_MATCH" 2> /dev/null); do
		exe=$(readlink "/proc/$pid/exe" 2> /dev/null) || continue
		case "$exe" in
		*/python*) printf '%s\n' "$pid" ;;
		esac
	done
}

# TSTP not STOP: catchable, so a hermes that traps it for its own graceful
# pause gets the chance to; if it doesn't trap it the default action is the
# same freeze either way.
hermes_pause() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	hermes_pids | xargs -r kill -TSTP 2> /dev/null
}
hermes_resume() {
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	hermes_pids | xargs -r kill -CONT 2> /dev/null
}

# Eco entry policy for remote access: an idle VNC session is turned off to
# save power; a connected VNC session stays alive so eco never drops the
# user mid-session; kdeconnectd is a separate service and is never touched
# here. Best-effort: a failure here must not fail the mode switch.
vnc_eco_off() {
	if [ -n "${MANGO_PM_TEST:-}" ]; then
		# The self-check counts hook firings through this trace file.
		[ -n "${VNC_ECO_TRACE:-}" ] && echo fired >> "$VNC_ECO_TRACE"
		return 0
	fi
	systemctl --user is-active --quiet wayvnc.service || return 0
	# A failed query is not the fact "zero clients". Keep VNC alive unless
	# wayvncctl answers and the client list is empty.
	VNC_CLIENTS=$(wayvncctl -j client-list 2> /dev/null) || return 0
	[ -z "$(printf '%s' "$VNC_CLIENTS" | jq -c '.[]' 2> /dev/null)" ] || return 0
	# In the background: the toggle stops units and reloads the bar, and a
	# caller such as battery-guard's low path must not stall on that while
	# it holds the mode lock at a critical charge.
	"$HOME/.config/ironbar/scripts/remote.sh" --toggle-vnc > /dev/null 2>&1 &
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

# Verified against /proc/<pid>/comm, not just "a pid is in the file" — a
# recycled pid must never be signalled.
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

# bars_restart() (waybar's own reload poke, USR1-to-bars.sh) is gone with
# waybar itself — mango-bard inotify-watches $MODE_FILE directly (T1) and
# needs no signal to pick up a mode change.

notify_mode() { # mode, detail
	[ -n "${MANGO_PM_TEST:-}" ] && return 0
	command -v notify-send > /dev/null 2>&1 || return 0
	icon='⚡'
	[ "$1" = eco ] && icon='🌿'
	[ "$1" = battery ] && icon='🔋'
	notify-send -a powermode -u low \
		-h string:x-canonical-private-synchronous:powermode \
		"$icon $1 mode" "${2:-}"
}

# ---------------------------------------------------------------- mode set

set_mode() { # mode
	m=$1
	prev_mode=$(current_mode)
	printf '%s' "$m" > "$MODE_FILE"
	apply_root "$m"
	case "$m" in
	eco) enter_dim "$PM_ECO_BRIGHT" ;;
	battery) enter_dim "$PM_BAT_BRIGHT" ;;
	*) exit_dim ;;
	esac
	if [ "$m" = eco ]; then
		hermes_pause
		drain_start
		# vnc_eco_off flips VNC off, so only fire on the eco entry edge —
		# re-applying eco while already in eco (weak(), a repeated auto())
		# must not flip it back on.
		[ "$prev_mode" = eco ] || vnc_eco_off
	else
		drain_stop
		hermes_resume
		docker_unpause
		ollama_restore
	fi
}

online() {
	[ -n "$AC" ] || AC=$(set -- /sys/class/power_supply/A[CD]*; printf '%s' "$1")
	[ -r "$AC/online" ] && cat "$AC/online" 2> /dev/null || printf 0
}

# Pure, so the self-check can assert the table directly — same shape as
# drain_decision()/docker_action() above.
mode_for() { # online pct eco_pct -> full|eco|battery
	if [ "$1" = 1 ]; then printf full
	elif [ "$2" -lt "$3" ]; then printf eco # unplugged already low: skip battery
	else printf battery
	fi
}

auto() {
	if [ -f "$WEAK_FILE" ]; then set_mode eco; return; fi
	[ -f "$MANUAL_FILE" ] && return # a click already decided; the cable doesn't override it
	set_mode "$(mode_for "$(online)" "$(battery_pct)" "$PM_BAT_ECO_PCT")"
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

force() { # mode, detail
	: > "$MANUAL_FILE"
	set_mode "$1"
	notify_mode "$1" "${2:-manual}"
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

# Sets the override and reapplies the current mode's payload — the ceiling is
# not itself a mode, so it doesn't touch $MODE_FILE. n == PM_CHARGE_LIMIT
# clears the override rather than storing a redundant copy, so a later
# powermode.conf edit is picked up on the next apply instead of being shadowed
# by a stale runtime file.
charge() { # n
	if [ "$1" = "$PM_CHARGE_LIMIT" ]; then rm -f "$CHARGE_FILE"
	else printf '%s' "$1" > "$CHARGE_FILE"
	fi
	apply_root "$(current_mode)"
}

status() {
	printf 'mode:   %s\n' "$(current_mode)"
	printf 'manual: %s\n' "$([ -f "$MANUAL_FILE" ] && echo yes || echo no)"
	printf 'weak:   %s\n' "$([ -f "$WEAK_FILE" ] && echo yes || echo no)"
	printf 'AC:     %s\n' "$([ "$(online)" = 1 ] && echo online || echo offline)"
	printf 'charge: %s%%\n' "$(charge_limit)"
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	MANGO_PM_TEST=1
	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT
	RUN="$T"
	MODE_FILE="$RUN/mode" MANUAL_FILE="$RUN/manual" BRIGHT_FILE="$RUN/bright" WEAK_FILE="$RUN/weak"
	DRAIN_PID="$RUN/drain.pid" OLLAMA_FILE="$RUN/ollama" CHARGE_FILE="$RUN/charge"
	LOCK_FILE="$RUN/lock"

	mkdir -p "$T/ac" "$T/bat"
	AC="$T/ac"
	MANGO_BAT_DIR="$T/bat"
	echo 1 > "$T/ac/online"
	echo 90 > "$T/bat/capacity" # healthy charge unless a case below says otherwise

	# AC online, nothing forced -> full
	auto
	[ "$(current_mode)" = full ] || { echo "auto on AC should pick full: $(current_mode)"; exit 1; }

	# unplug at a healthy charge -> battery, not eco
	echo 0 > "$T/ac/online"
	auto
	[ "$(current_mode)" = battery ] || { echo "auto off AC at 90% should pick battery: $(current_mode)"; exit 1; }

	# unplug already low -> straight to eco, skipping battery
	echo 30 > "$T/bat/capacity"
	rm -f "$MANUAL_FILE"
	auto
	[ "$(current_mode)" = eco ] || { echo "auto off AC at 30% should pick eco: $(current_mode)"; exit 1; }
	echo 90 > "$T/bat/capacity"

	# a manual override survives a repeated auto() with the cable unchanged
	toggle # eco -> full, sets manual
	[ "$(current_mode)" = full ] || { echo "toggle should have flipped to full"; exit 1; }
	[ -f "$MANUAL_FILE" ] || { echo "toggle should set the manual marker"; exit 1; }
	auto # still on battery (online=0), but manual should win
	[ "$(current_mode)" = full ] || { echo "auto must not override a manual choice"; exit 1; }

	# a cable edge clears manual and re-decides
	cable
	[ "$(current_mode)" = battery ] || { echo "cable edge should clear manual and re-decide from AC state: $(current_mode)"; exit 1; }
	[ -f "$MANUAL_FILE" ] && { echo "cable edge should have cleared the manual marker"; exit 1; }

	# mode_for table, directly
	[ "$(mode_for 1 90 40)" = full ] || { echo "mode_for online should be full regardless of charge"; exit 1; }
	[ "$(mode_for 0 90 40)" = battery ] || { echo "mode_for offline, above the floor, should be battery"; exit 1; }
	[ "$(mode_for 0 39 40)" = eco ] || { echo "mode_for offline, under the floor, should be eco"; exit 1; }
	[ "$(mode_for 0 40 40)" = battery ] || { echo "mode_for exactly at the floor should still be battery"; exit 1; }

	# low: forces eco and sticks, same as a manual click
	force battery
	[ "$(current_mode)" = battery ] || { echo "force battery failed"; exit 1; }
	force eco 'battery low'
	[ "$(current_mode)" = eco ] || { echo "force eco (low) failed"; exit 1; }
	[ -f "$MANUAL_FILE" ] || { echo "low should set the manual marker so it doesn't flap back to battery"; exit 1; }

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

	# the VNC-off hook fires once per entry into eco, never on re-application
	# (a second fire would toggle VNC back on)
	VNC_ECO_TRACE="$T/vnc-eco"
	force eco
	force eco # already eco: must not fire again
	[ "$(grep -c fired "$VNC_ECO_TRACE" 2> /dev/null)" = 1 ] || { echo "vnc_eco_off must fire exactly once per eco entry"; exit 1; }
	force full
	force eco # a fresh entry fires again
	[ "$(grep -c fired "$VNC_ECO_TRACE")" = 2 ] || { echo "vnc_eco_off must fire again on a fresh eco entry"; exit 1; }
	force full
	unset VNC_ECO_TRACE

	# payload shape: every key present, mode-appropriate values
	P=$(payload eco)
	printf '%s\n' "$P" | grep -qx 'MODE=eco' || { echo "eco payload missing MODE"; exit 1; }
	printf '%s\n' "$P" | grep -qx "EPP=$PM_ECO_EPP" || { echo "eco payload missing EPP"; exit 1; }
	P=$(payload full)
	printf '%s\n' "$P" | grep -qx 'MODE=full' || { echo "full payload missing MODE"; exit 1; }
	P=$(payload battery)
	printf '%s\n' "$P" | grep -qx 'MODE=battery' || { echo "battery payload missing MODE"; exit 1; }
	printf '%s\n' "$P" | grep -qx "EPP=$PM_BAT_EPP" || { echo "battery payload should use PM_BAT_EPP, not eco's"; exit 1; }
	printf '%s\n' "$P" | grep -qx "NO_TURBO=$PM_BAT_NO_TURBO" || { echo "battery payload should use PM_BAT_NO_TURBO"; exit 1; }
	printf '%s\n' "$P" | grep -qx "PCI_PM=$PM_ECO_PCI_PM" || { echo "battery payload should reuse eco's I/O-side PCI_PM"; exit 1; }

	# charge ceiling: config default with no override, then the travel override,
	# present in every mode's payload since it's emitted outside the case
	printf '%s\n' "$(payload full)" | grep -qx "CHARGE_LIMIT=$PM_CHARGE_LIMIT" || { echo "payload should default to PM_CHARGE_LIMIT with no override"; exit 1; }
	charge 100
	[ "$(cat "$CHARGE_FILE")" = 100 ] || { echo "charge 100 should write the override file"; exit 1; }
	printf '%s\n' "$(payload eco)" | grep -qx 'CHARGE_LIMIT=100' || { echo "payload should reflect the override in every mode"; exit 1; }
	# an unplug/replug must not discard a travel override
	cable
	[ "$(cat "$CHARGE_FILE")" = 100 ] || { echo "cable edge must not clear the charge override"; exit 1; }
	# setting it back to the config value clears the override file instead of
	# storing a redundant copy
	charge "$PM_CHARGE_LIMIT"
	[ -f "$CHARGE_FILE" ] && { echo "charge back to PM_CHARGE_LIMIT should clear the override"; exit 1; }

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

# Every mode-changing verb funnels through set_mode() (auto/cable via
# mode_for(), toggle/force/weak directly), so one lock acquired here, before
# any of them run, serializes the whole call — set_mode()'s own MODE_FILE
# write and its enter_dim()/exit_dim() BRIGHT_FILE first-touch both included.
# Held for the rest of this process (no explicit unlock: the fd, and so the
# lock, closes when the script exits), which is exactly the span a bouncing
# AC edge needs serialized. `drain` is its own detached process
# (drain_start's `setsid "$0" drain &`) that never calls set_mode again, so
# it is deliberately not in this list — locking it here would just make the
# eco wait/pause loop hold the lock for however long it runs, with no payoff.
case "${1:-}" in
auto | cable | toggle | eco | full | battery | low | weak | unweak)
	exec 9> "$LOCK_FILE"
	flock -x 9
	;;
esac

case "${1:-}" in
auto) auto ;;
cable) cable ;;
toggle) toggle ;;
eco) force eco ;;
full) force full ;;
battery) force battery ;;
low) force eco 'battery low' ;;
weak) weak ;;
unweak) unweak ;;
charge) charge "${2:?usage: powermode.sh charge N}" ;;
drain) drain ;;
status) status ;;
*)
	echo "usage: powermode.sh auto|cable|toggle|eco|full|battery|low|weak|unweak|charge N|drain|status|test" >&2
	exit 1
	;;
esac
