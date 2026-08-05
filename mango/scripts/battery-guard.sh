#!/usr/bin/env bash
# Low-battery failsafe: warn, then alarm, then suspend before the firmware or
# UPower does something blunter.
#
# waybar/scripts/battery.sh already turns the bar glyph red at 20%, but a
# recoloured glyph is not a failsafe — it is silent, and it is invisible if the
# bar is covered or you are not looking at it. Below that there was nothing at
# all until UPower's PercentageAction=2.0. On this machine hibernate does not
# work (4 GB swap against 46 GB RAM, no resume= on the cmdline), so UPower's
# CriticalPowerAction=Auto resolves to *power off* — an unclean session kill at
# 2%. Suspending at 5% means that backstop is never reached while awake.
#
#   battery-guard.sh        exec-once from mango/config.conf — poll and act
#   battery-guard.sh test   assert the tier thresholds, no device needed
set -uo pipefail

BAT="${MANGO_BAT_DIR:-}"
AC="${MANGO_AC_DIR:-}"

# warn,critical,action,floor — same one-env-var-holds-the-tuple shape as
# MANGO_POMODORO in waybar/scripts/clock.sh.
IFS=, read -r WARN CRIT ACT FLOOR <<< "${MANGO_BATTERY:-20,10,5,3}"

DRYRUN="${MANGO_BAT_DRYRUN:-}"

POLL=30    # seconds between reads; 30s is well inside the time 1% takes to burn
GRACE=60   # countdown before the suspend actually fires
RENAG=300  # re-sound the critical alarm this often while it is ignored
SETTLE=120 # after resuming, wait this long before considering suspend again

[ -n "$DRYRUN" ] && GRACE=6  # so the ladder can be watched end to end in a terminal

BUSY="$(dirname "$0")/busy.sh"

# ---------------------------------------------------------------- primitives

readf() { [ -r "$BAT/$1" ] && cat "$BAT/$1" 2>/dev/null || true; }

# clock.sh's pair, verbatim in spirit: replace-in-place instead of stacking, and
# audio in a subshell so a wedged sink cannot stall the poll loop.
#
# mako sets ignore-timeout=1 globally, so -t is a no-op here — urgency is the
# only lever. Its [urgency=critical] block sets default-timeout=0, which is what
# makes the low-battery notification stick on screen until dismissed. That
# persistent, error-bordered notification is the visual half of the failsafe.
notify() { # urgency summary body
	notify-send -a battery -u "$1" \
		-h string:x-canonical-private-synchronous:battery-guard "$2" "$3"
}

beep() { # sound file basename
	local f=/usr/share/sounds/freedesktop/stereo/$1.oga
	[ -r "$f" ] && command -v paplay > /dev/null 2>&1 && (paplay "$f" > /dev/null 2>&1 &)
}

# Severity ladder. Charging at any percentage is not an emergency, so it short
# circuits before the thresholds are consulted at all.
tier() { # pct status -> none|warn|crit|act|floor
	case "$2" in Charging | Full) printf 'none\n'; return ;; esac
	if [ "$1" -le "$FLOOR" ]; then printf 'floor\n'
	elif [ "$1" -le "$ACT" ]; then printf 'act\n'
	elif [ "$1" -le "$CRIT" ]; then printf 'crit\n'
	elif [ "$1" -le "$WARN" ]; then printf 'warn\n'
	else printf 'none\n'
	fi
}

rank() { case "$1" in none) echo 0 ;; warn) echo 1 ;; crit) echo 2 ;; act) echo 3 ;; floor) echo 4 ;; esac; }
thr() { case "$1" in warn) echo "$WARN" ;; crit) echo "$CRIT" ;; act) echo "$ACT" ;; floor) echo "$FLOOR" ;; *) echo 101 ;; esac; }

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	WARN=20 CRIT=10 ACT=5 FLOOR=3

	for case in 100:none 21:none 20:warn 15:warn 11:warn 10:crit 6:crit \
		5:act 4:act 3:floor 1:floor 0:floor; do
		got=$(tier "${case%%:*}" Discharging)
		[ "$got" = "${case##*:}" ] || { echo "tier ${case%%:*} = $got, want ${case##*:}"; exit 1; }
	done

	# A battery on the mains is never an emergency, however low it reads —
	# without this the guard would suspend a laptop that is actively charging.
	for p in 0 3 5 10 20 99; do
		for s in Charging Full; do
			[ "$(tier "$p" "$s")" = none ] || { echo "$s at $p% should be none"; exit 1; }
		done
	done

	# Unknown/Not charging must still be treated as draining — some firmware
	# reports "Unknown" on the way down.
	[ "$(tier 4 Unknown)" = act ] || { echo "Unknown status should still act"; exit 1; }
	[ "$(tier 4 "Not charging")" = act ] || { echo "Not charging should still act"; exit 1; }

	# severity must be strictly ordered, or the fire-once logic re-fires forever
	[ "$(rank none)" -lt "$(rank warn)" ] && [ "$(rank warn)" -lt "$(rank crit)" ] &&
		[ "$(rank crit)" -lt "$(rank act)" ] && [ "$(rank act)" -lt "$(rank floor)" ] ||
		{ echo "rank order wrong"; exit 1; }

	# the re-arm point has to sit above the threshold that fired, else the tier
	# flaps on and off across a single percent
	[ "$(thr warn)" -eq 20 ] && [ "$(thr floor)" -eq 3 ] && [ "$(thr none)" -eq 101 ] ||
		{ echo "thr wrong"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- actions

# Returns 0 once the machine went to sleep (or would have), 1 if the attempt was
# called off — either because something is mid-flight or because AC came back.
attempt_suspend() { # tier pct
	local t=$1 pct=$2 reason left

	# Above the hard floor, an in-flight package transaction or download wins:
	# waking to a half-applied pacman transaction is worse than losing another
	# percent. At or below the floor there is no more percent to spend.
	if [ "$t" != floor ] && reason=$("$BUSY"); then
		notify critical "Battery ${pct}% — suspend deferred" \
			"Waiting on $reason. Suspending at ${FLOOR}% regardless."
		beep alarm-clock-elapsed
		return 1
	fi

	# Six checks across the countdown, whatever the countdown is — the point of
	# the loop is to notice a charger being plugged in, so the step has to
	# follow GRACE rather than be a constant that outlasts it.
	local step=$((GRACE / 6))
	[ "$step" -lt 1 ] && step=1

	left=$GRACE
	while [ "$left" -gt 0 ]; do
		case "$(readf status)" in
		Charging | Full)
			notify normal "Battery charging" "Suspend cancelled."
			return 1
			;;
		esac
		notify critical "Battery ${pct}% — suspending in ${left}s" \
			"Plug in or save your work now."
		beep alarm-clock-elapsed
		sleep "$step"
		left=$((left - step))
	done

	notify critical "Battery ${pct}% — suspending" "Out of headroom."
	if [ -n "$DRYRUN" ]; then
		echo "DRYRUN: would run systemctl suspend at ${pct}%"
	else
		systemctl suspend || loginctl suspend
	fi
	return 0
}

# ---------------------------------------------------------------- main

[ -n "$BAT" ] || BAT=$(set -- /sys/class/power_supply/BAT*; printf '%s' "$1")
[ -n "$AC" ] || AC=$(set -- /sys/class/power_supply/A[CD]*; printf '%s' "$1")

# Desktops have no BAT* at all — leave rather than poll a directory forever.
[ -d "$BAT" ] && [ -r "$BAT/capacity" ] || exit 0

LAST=none      # most severe tier already announced
REARM=101      # charge at which LAST is forgotten, so tiers cannot flap
CRIT_AT=0      # when the critical alarm last sounded

while :; do
	PCT=$(readf capacity)
	STATUS=$(readf status)
	NOW=$SECONDS

	# A removed or unreadable battery is not an emergency either.
	if [ -z "$PCT" ]; then sleep "$POLL"; continue; fi

	case "$STATUS" in
	Charging | Full) LAST=none REARM=101 ;;
	*) [ "$PCT" -ge "$REARM" ] && { LAST=none REARM=101; } ;;
	esac

	T=$(tier "$PCT" "$STATUS")
	NEW=$([ "$(rank "$T")" -gt "$(rank "$LAST")" ] && echo 1 || echo 0)

	case "$T" in
	warn)
		if [ "$NEW" = 1 ]; then
			notify normal "Battery low — ${PCT}%" "Plug in soon."
			beep dialog-warning
		fi
		;;
	crit)
		# Unlike warn, this one nags: it is the last tier where you still have
		# time to do something about it by hand.
		if [ "$NEW" = 1 ] || [ $((NOW - CRIT_AT)) -ge "$RENAG" ]; then
			notify critical "Battery critical — ${PCT}%" \
				"Suspending automatically at ${ACT}%."
			beep alarm-clock-elapsed
			CRIT_AT=$NOW
		fi
		;;
	act | floor)
		# Retried every poll rather than fired once: a deferred attempt has to
		# come back around once the download or the pacman lock clears.
		if attempt_suspend "$T" "$PCT"; then
			[ -n "$DRYRUN" ] && exit 0
			# Back from sleep. Give the user a window to reach a charger before
			# offering to suspend again, or resuming becomes pointless.
			sleep "$SETTLE"
			LAST=none REARM=101
			continue
		fi
		;;
	esac

	[ "$(rank "$T")" -gt "$(rank "$LAST")" ] && { LAST=$T; REARM=$(($(thr "$T") + 2)); }

	sleep "$POLL"
done
