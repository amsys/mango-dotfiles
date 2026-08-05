#!/bin/bash
# NetworkManager -> waybar bridge. Two jobs, one loop:
#
#   1. Refresh the three network modules the instant NM reports a change,
#      instead of letting them wait out their 5s poll. That skew is why the
#      security lock used to flip seconds before the signal readout caught up.
#   2. Draw a busy spinner in custom/wifi's slot for the whole of any state
#      change — manual join, auto-connect, roam, disconnect — while net.sh
#      hides custom/wifi so the spinner *replaces* the arc rather than sitting
#      beside it.
#
# This is a *continuous* waybar module (`custom/netwatch`, no interval): stdout
# is the module text, one line per frame, empty line = hidden. That is what
# makes the animation free — waybar is already reading this pipe, so a frame
# costs one write. Signalling waybar per frame instead would re-exec
# `net.sh --wifi` (188ms) and `--eth` at 8Hz, which is unaffordable.
#
# ponytail: no state file, no pidfile, no lock. waybar owns the lifecycle, so
# exiting on EOF and letting `restart-interval` respawn us is the whole
# supervision story.
set -u

DEV="${MANGO_WIFI_DEV:-wlo1}"
TICK=0.12 # spinner frame duration while busy
# Idle wakeup. NM events end the read the moment they arrive, so this only
# bounds how long wifi-menu.sh's USR1 waits: bash runs the trap but *restarts*
# the read, so SCAN=1 is invisible until the current timeout expires. Half a
# second of latency on a scan spinner, at the price of two no-op reads per
# second and zero forks.
IDLE=0.5

# `nmcli monitor` emits a burst of 5-10 lines per transition, 1-3ms apart.
# Refreshing on each would re-exec net.sh ~20x per join, so coalesce: refresh
# once the burst has been quiet for this long.
DEBOUNCE=0.15

# Eight-frame pie loader: md-circle_slice_1..8, U+F0AA5..U+F0AAC. Private-use
# area, so invisible in an editor — written as escapes like net.sh's icons.
# Deliberately from the same Nerd Font that renders net.sh's signal arcs: the
# spinner stands in for the arc, so the advance width has to match.
FRAMES=$(printf '\xf3\xb0\xaa\xa5 \xf3\xb0\xaa\xa6 \xf3\xb0\xaa\xa7 \xf3\xb0\xaa\xa8 \xf3\xb0\xaa\xa9 \xf3\xb0\xaa\xaa \xf3\xb0\xaa\xab \xf3\xb0\xaa\xac')

# Sets $FRAME rather than printing it: `$(next_frame)` would rotate the list in
# a subshell and hand back the same glyph forever.
next_frame() { # pop the head glyph, push it back on the tail
	FRAME=${FRAMES%% *}
	FRAMES="${FRAMES#* } $FRAME"
}

# 10 = netsec, 12 = wifi + eth. Always fired together, so the security verdict
# and the signal readout can never disagree by a poll interval.
refresh_bar() {
	pkill -RTMIN+10 waybar 2>/dev/null
	pkill -RTMIN+12 waybar 2>/dev/null
	return 0
}

# `nmcli monitor` carries the device state in the line itself — "wlo1:
# connecting (prepare)", "wlo1: deactivating", "wlo1: connected" — so the
# spinner never has to fork nmcli to ask. Returns:
#   0 busy   1 idle   2 line says nothing about this device's state
# (2 covers "wlo1: using connection '...'" and every non-device line.)
line_state() {
	case "$1" in
	"$DEV: connecting"* | "$DEV: deactivating"*) return 0 ;;
	"$DEV: connected"* | "$DEV: disconnected"* | \
		"$DEV: unavailable"* | "$DEV: unmanaged"* | "$DEV: unmanageable"*) return 1 ;;
	*) return 2 ;;
	esac
}

# Only used once, at startup: waybar may launch us mid-transition.
dev_busy_now() {
	case "$(nmcli -g GENERAL.STATE device show "$DEV" 2>/dev/null)" in
	40\ * | 50\ * | 60\ * | 70\ * | 80\ * | 90\ * | 110\ *) return 0 ;;
	*) return 1 ;;
	esac
}

selftest() {
	DEV=wlo1 # the canned lines below are verbatim from this machine
	fail=0
	t() { # expected: busy | idle | ignore
		line_state "$2"
		case "$?:$1" in
		0:busy | 1:idle | 2:ignore) ;;
		*)
			printf 'FAIL: %-42s should be %s\n' "$2" "$1"
			fail=1
			;;
		esac
	}
	# Verbatim from `nmcli monitor` during a disconnect/reconnect cycle.
	t busy 'wlo1: deactivating'
	t busy 'wlo1: connecting (prepare)'
	t busy 'wlo1: connecting (configuring)'
	t busy 'wlo1: connecting (getting IP configuration)'
	t busy 'wlo1: connecting (checking IP connectivity)'
	t busy 'wlo1: connecting (starting secondary connections)'
	t idle 'wlo1: connected'
	t idle 'wlo1: disconnected'
	t idle 'wlo1: unavailable'
	t ignore "wlo1: using connection 'Albania Tirana - GREEN-FLOOR 3 E'"
	t ignore 'NetworkManager is running'
	t ignore "Connectivity is now 'full'"
	t ignore 'eno2: connecting (prepare)' # another device must not spin ours
	t ignore "There's no primary connection"

	# 8-periodic or the pie stutters on wrap.
	next_frame
	first=$FRAME
	i=1
	while [ "$i" -le 8 ]; do
		next_frame
		i=$((i + 1))
	done
	[ "$FRAME" = "$first" ] || {
		printf 'FAIL: frame rotation is not 8-periodic\n'
		fail=1
	}

	[ "$fail" -eq 0 ] && printf 'net-watch: all checks passed\n'
	return "$fail"
}

case "${1:-}" in
test | --selftest)
	selftest
	exit $?
	;;
esac

# A pipeline would put the loop in a subshell, where the USR1/USR2 traps
# wifi-menu.sh relies on are unreachable; coproc keeps everything in this shell
# and hands back a pid, so `nmcli monitor` cannot outlive us.
# `exec` matters: without it the brace group stays a bash subshell that forks
# nmcli, $NM_PID names the subshell, and killing it orphans a live `nmcli
# monitor` on every waybar restart.
coproc NM { exec nmcli monitor 2>/dev/null; }
# bash unsets NM_PID as soon as the coproc reaps, so guard it — under `set -u`
# a bare expansion turns the exit trap into an error and orphans the monitor.
trap '[ -n "${NM_PID:-}" ] && kill "$NM_PID" 2>/dev/null; exit 0' EXIT INT TERM

# A rescan changes no device state, so NM emits nothing for it — wifi-menu.sh
# asks for the spinner by hand around its blocking `nmcli device wifi list`.
SCAN=0
trap 'SCAN=1' USR1
trap 'SCAN=0' USR2

BUSY=0
dev_busy_now && BUSY=1
PEND=1 # refresh once on startup so the bar matches reality immediately
SHOWN=x
TICKS=0

while :; do
	if [ "$BUSY" = 1 ] || [ "$SCAN" = 1 ]; then
		TMO=$TICK
	elif [ "$PEND" = 1 ]; then
		TMO=$DEBOUNCE
	else
		TMO=$IDLE
	fi

	IFS= read -r -u "${NM[0]}" -t "$TMO" LINE
	rc=$?

	if [ "$rc" = 0 ]; then # an NM event
		line_state "$LINE"
		case $? in
		0) BUSY=1 ;;
		1) BUSY=0 ;;
		esac
		PEND=1
		continue # burst still arriving; hold the refresh and the frame
	fi

	# read(1) returns >128 on timeout (or an interrupting signal) and 1 on
	# EOF. EOF means NetworkManager restarted — exit and let waybar's
	# restart-interval bring us back with a fresh monitor.
	[ "$rc" -le 128 ] && exit 0

	if [ "$PEND" = 1 ]; then
		refresh_bar
		PEND=0
	fi

	if [ "$BUSY" = 1 ]; then
		# Belt and braces: a dropped "connected" event would otherwise leave
		# this spinning forever. Ask NM directly every ~2s of animation — half
		# a fork per second, and only while something is actually happening.
		TICKS=$((TICKS + 1))
		if [ $((TICKS % 16)) = 0 ]; then
			dev_busy_now || BUSY=0
		fi
	else
		TICKS=0
	fi

	if [ "$BUSY" = 1 ] || [ "$SCAN" = 1 ]; then
		next_frame
		OUT=$FRAME
	else
		OUT=""
	fi
	if [ "$OUT" != "$SHOWN" ]; then
		SHOWN=$OUT
		# Same wrapper every icon-bearing module uses (net.sh's barico), so the
		# spinner inherits the arc's size and baseline exactly. An empty line
		# stays empty — that is what hides the module.
		if [ -n "$OUT" ]; then
			printf '<span size="115%%" rise="-1200">%s</span>\n' "$OUT"
		else
			printf '\n'
		fi
	fi
done
