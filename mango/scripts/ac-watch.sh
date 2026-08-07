#!/usr/bin/env bash
# Chime and notify the moment the AC adapter goes in or comes out. Also the
# only trigger for switching power mode off the cable: it already sees the
# edge instantly (see below for why that matters), so mango/scripts/powermode.sh
# rides the same udev event rather than getting a second poll of its own.
#
# battery-guard.sh already sees this transition — it polls the battery every 30s
# and already resolves $AC — so this could have been six lines inside its loop.
# It is separate because the whole value of a plug confirmation is that it is
# prompt: the reason to want one at all is catching a dead brick, a half-seated
# jack, or a port that has quietly stopped negotiating charge, and an answer
# fifteen seconds after the fact catches none of them. Dropping that guard's
# POLL to fix it would multiply the wakeups of a failsafe for a cosmetic reason.
#
# So: udev events rather than a poll, and battery-guard stays a failsafe that
# does one thing.
#
#   ac-watch.sh        exec-once from mango/config.conf — watch and announce
#   ac-watch.sh test   assert the wording and the edge detector, no device needed
set -uo pipefail

AC="${MANGO_AC_DIR:-}"
BAT="${MANGO_BAT_DIR:-}"
POWERMODE="$(dirname "$0")/powermode.sh"

RETRY=5 # before rebuilding the event source, if systemd-udevd goes away

# ---------------------------------------------------------------- primitives

# battery-guard.sh's pair, with one addition. Replace-in-place rather than
# stack, so a fast unplug-replug leaves one notification and not three; audio in
# a background subshell so a wedged sink cannot stall the watch loop.
#
# -u low deliberately: mako's [urgency=low] block gives this 3000ms and dimmer
# text, which is what an informational notification should get. -t would do
# nothing anyway — the mako template sets ignore-timeout=1 globally, so urgency
# is the only lever there is.
notify() { # summary body
	notify-send -a battery -u low \
		-h string:x-canonical-private-synchronous:ac-watch "$1" "$2"
}

# wpctl prints `Volume: 0.92`, and appends ` [MUTED]` when the sink is muted.
# No wpctl and no sink both read as "not muted" — beep()'s own guards are what
# keep a machine with no sound theme quiet.
muted() {
	command -v wpctl > /dev/null 2>&1 &&
		wpctl get-volume @DEFAULT_AUDIO_SINK@ 2> /dev/null | grep -q MUTED
}

beep() { # sound file basename
	local f=/usr/share/sounds/freedesktop/stereo/$1.oga
	muted && return 0
	[ -r "$f" ] && command -v paplay > /dev/null 2>&1 && (paplay "$f" > /dev/null 2>&1 &)
}

# ---------------------------------------------------------------- wording

# Both are driven by `online` rather than by $BAT/status: ACPI lags the
# transition by a beat, so status can still read Discharging for a moment after
# the plug goes in — which would put "discharging" under "Charger connected".
summary() { # online
	case "$1" in
	1) printf 'Charger connected\n' ;;
	*) printf 'On battery\n' ;;
	esac
}

body() { # online pct
	[ -n "$2" ] || return 0
	case "$1" in
	1) printf '%s%% · charging\n' "$2" ;;
	*) printf '%s%% · discharging\n' "$2" ;;
	esac
}

# An edge, not an event. The stream also carries udevadm's own two-line startup
# banner and a `change` for BAT0 on every capacity tick — same subsystem, so
# those match the filter too. Reading `online` back and comparing absorbs all of
# it without having to parse anything.
changed() { # prev now
	[ -n "$2" ] && [ "$2" != "$1" ]
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	[ "$(summary 1)" = "Charger connected" ] || { echo "plug summary wrong"; exit 1; }
	[ "$(summary 0)" = "On battery" ] || { echo "unplug summary wrong"; exit 1; }

	[ "$(body 1 73)" = "73% · charging" ] || { echo "plug body wrong"; exit 1; }
	[ "$(body 0 73)" = "73% · discharging" ] || { echo "unplug body wrong"; exit 1; }

	# An unreadable capacity has to degrade to an empty body, not to "% · charging".
	[ -z "$(body 1 '')" ] || { echo "missing capacity should give an empty body"; exit 1; }

	# The two edges...
	changed 0 1 || { echo "0->1 is an edge"; exit 1; }
	changed 1 0 || { echo "1->0 is an edge"; exit 1; }

	# ...and the three non-edges the stream actually delivers. The 0->0 case is
	# the load-bearing one: BAT0 ticks its capacity through this same filter
	# every couple of minutes, and each one re-reads `online`.
	! changed 1 1 || { echo "1->1 must not announce"; exit 1; }
	! changed 0 0 || { echo "0->0 must not announce (BAT0 capacity ticks)"; exit 1; }
	! changed 0 '' || { echo "an unreadable online must not announce"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- main

# Same discovery idiom as battery-guard.sh and waybar/scripts/battery.sh, so
# MANGO_AC_DIR/MANGO_BAT_DIR stay shared across all three.
[ -n "$AC" ] || AC=$(set -- /sys/class/power_supply/A[CD]*; printf '%s' "$1")
[ -n "$BAT" ] || BAT=$(set -- /sys/class/power_supply/BAT*; printf '%s' "$1")

# No mains device means there is nothing to announce — a desktop, or the glob
# not matching. Leave rather than watch a stream forever for nobody.
[ -r "$AC/online" ] || exit 0

online() { cat "$AC/online" 2> /dev/null; }
capacity() { [ -r "$BAT/capacity" ] && cat "$BAT/capacity" 2> /dev/null || true; }

PREV=$(online)
"$POWERMODE" cable > /dev/null 2>&1 &  # mode has to be right at login too, not just on the next edge

# The outer loop is not decoration: `read` hits EOF if systemd-udevd restarts,
# which happens on any routine system update. exec-once has no supervisor behind
# it — waybar's restart-interval covers its --watch modules, nothing covers this
# — so without the retry the chime would die silently and stay dead until the
# next login. PREV is deliberately *not* reset across a restart, so a cable that
# moved while the source was down is announced late rather than never.
while :; do
	# coproc rather than `udevadm ... | while read`: in a pipeline the stream
	# producer is a sibling process, so killing this script would orphan
	# udevadm. That is the leak waybar/scripts/watch.sh exists to avoid, and
	# that mango-window.sh still demonstrates.
	coproc EV { exec udevadm monitor --udev --subsystem-match=power_supply 2> /dev/null; }
	# Captured once: bash drops EV the moment the coproc reaps, and a bare
	# ${EV[0]} in the read below would then take the whole script out under -u.
	EVFD=${EV[0]}
	trap '[ -n "${EV_PID:-}" ] && kill "$EV_PID" 2> /dev/null; exit 0' EXIT INT TERM

	while IFS= read -r -u "$EVFD" _; do
		NOW=$(online)
		changed "$PREV" "$NOW" || continue
		PREV=$NOW

		notify "$(summary "$NOW")" "$(body "$NOW" "$(capacity)")"
		if [ "$NOW" = 1 ]; then beep power-plug; else beep power-unplug; fi
		"$POWERMODE" cable > /dev/null 2>&1 &
	done

	[ -n "${EV_PID:-}" ] && kill "$EV_PID" 2> /dev/null
	sleep "$RETRY"
done
