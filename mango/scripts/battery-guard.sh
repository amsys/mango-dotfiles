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

POWERMODE="$(dirname "$0")/powermode.sh"
# Same file mango/scripts/powermode.sh calls WEAK_FILE — read here only to
# tell whether it's already latched, so a sustained weak charger doesn't
# re-notify every 30s poll. powermode.sh remains the sole writer.
WEAK_MARK="${XDG_RUNTIME_DIR:-/tmp}/mango-powermode.weak"
CONF="${MANGO_POWERMODE_CONF:-$HOME/.config/mango/powermode.conf}"
[ -r "$CONF" ] && . "$CONF"
PM_WEAK_MIN_W=${PM_WEAK_MIN_W:-45}
PM_WEAK_POLLS=${PM_WEAK_POLLS:-2}

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

# A charger is plugged in but losing the race against the load — same symptom
# as no charger at all, just easier to miss because the icon still says
# "charging" territory. "Not charging" is deliberately excluded: that's a full
# battery on a healthy charger topping off, not a weak one.
weak_on_ac() { # status, online -> true if AC reports online but the battery is still draining
	[ "$2" = 1 ] && [ "$1" = Discharging ]
}

# A USB-C source negotiating less than $1 watts — catches a phone brick
# immediately, before net drain would ever show it. Globbed rather than
# resolved once: which ucsi-source-psy-* is live can change across a
# replug. MANGO_PM_UCSI_GLOB overrides the glob for the self-check.
weak_ucsi() { # min-watts -> true if any live USB-C source is under it
	for d in ${MANGO_PM_UCSI_GLOB:-/sys/class/power_supply/ucsi-source-psy-*}; do
		[ -d "$d" ] || continue
		[ "$(cat "$d/online" 2> /dev/null)" = 1 ] || continue
		vmax=$(cat "$d/voltage_max" 2> /dev/null) imax=$(cat "$d/current_max" 2> /dev/null)
		[ -n "$vmax" ] && [ -n "$imax" ] && [ "$vmax" -gt 0 ] 2> /dev/null && [ "$imax" -gt 0 ] 2> /dev/null || continue
		w=$(awk -v v="$vmax" -v i="$imax" 'BEGIN { printf "%.0f", v * i / 1e12 }')
		[ "$w" -lt "$1" ] && return 0
	done
	return 1
}

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

	# --- weak charger ---
	weak_on_ac Discharging 1 || { echo "AC online + Discharging should read weak"; exit 1; }
	weak_on_ac Charging 1 && { echo "Charging on AC must not read weak"; exit 1; }
	weak_on_ac "Not charging" 1 && { echo "Not charging (full battery, topped off) must not read weak"; exit 1; }
	weak_on_ac Discharging 0 && { echo "no AC at all is not a weak-charger condition"; exit 1; }

	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT
	mkdir -p "$T/weak"
	printf '1\n' > "$T/weak/online"
	printf '5000000\n' > "$T/weak/voltage_max"  # 5V
	printf '2000000\n' > "$T/weak/current_max"  # 2A -> 10W, under a 45W laptop charger
	MANGO_PM_UCSI_GLOB="$T/weak"
	weak_ucsi 45 || { echo "a 10W source should read weak against a 45W floor"; exit 1; }
	weak_ucsi 5 && { echo "a 10W source should NOT read weak against a 5W floor"; exit 1; }

	printf '0\n' > "$T/weak/online"
	weak_ucsi 45 && { echo "an offline USB-C port must not count"; exit 1; }

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
WEAKN=0        # consecutive polls that read as a weak charger
RECN=0         # consecutive healthy polls since a weak latch, before it's cleared

while :; do
	PCT=$(readf capacity)
	STATUS=$(readf status)
	NOW=$SECONDS

	# A removed or unreadable battery is not an emergency either.
	if [ -z "$PCT" ]; then sleep "$POLL"; continue; fi

	# Weak-charger check runs independently of the low-battery ladder below —
	# it can fire at 90% just as well as at 15%, the symptom is the charger,
	# not the level. WEAK_MARK is powermode.sh's own latch file; this only
	# reads it, to avoid re-notifying every 30s while it stays set.
	ONLINE=$(cat "$AC/online" 2> /dev/null || echo 0)
	if weak_on_ac "$STATUS" "$ONLINE" || weak_ucsi "$PM_WEAK_MIN_W"; then
		WEAKN=$((WEAKN + 1))
		RECN=0
		if [ "$WEAKN" -ge "$PM_WEAK_POLLS" ] && [ ! -f "$WEAK_MARK" ]; then
			notify critical "Weak charger — ${PCT}%" \
				"Plugged in but still losing charge. Switched to eco mode."
			beep dialog-warning
			"$POWERMODE" weak
		fi
	else
		WEAKN=0
		if [ -f "$WEAK_MARK" ]; then
			case "$STATUS" in
			Charging | Full) RECN=$((RECN + 1)) ;;
			*) RECN=0 ;;
			esac
			if [ "$RECN" -ge 2 ]; then
				notify normal "Charger recovered" "Back to full performance."
				"$POWERMODE" unweak
				RECN=0
			fi
		fi
	fi

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
