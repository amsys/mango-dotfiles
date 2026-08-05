#!/bin/sh
# Battery indicator for waybar, with a health/wear tooltip.
#
# Replaces the built-in `battery` module: its tooltip can only interpolate
# waybar's own placeholders, so there is no way to draw the ██░░ meters the
# rest of the bar's tooltips use, and no way to show wear against the design
# capacity — the number that actually tells you when to buy a new cell.
#
#   battery.sh          custom/battery exec — emit JSON
#   battery.sh test     assert the arithmetic against a canned sysfs tree
set -u

. "$(dirname "$0")/tooltip.sh"

BAT="${MANGO_BAT_DIR:-}"
AC="${MANGO_AC_DIR:-}"

# Material Symbols Rounded, same 115%/-1200 wrapper as every other bar icon.
# battery_0_bar .. battery_6_bar are not contiguous, hence the table.
ic_chg() { printf '\xee\x86\xa3'; }  # battery_charging_full   U+E1A3
ic_bat() { # capacity -> the matching fill level
	case $(( ($1 + 8) / 17 )) in
	0) printf '\xee\xaf\x9c' ;;  # battery_0_bar  U+EBDC
	1) printf '\xef\x82\x9c' ;;  # battery_1_bar  U+F09C
	2) printf '\xef\x82\x9d' ;;  # battery_2_bar  U+F09D
	3) printf '\xef\x82\x9e' ;;  # battery_3_bar  U+F09E
	4) printf '\xef\x82\x9f' ;;  # battery_4_bar  U+F09F
	5) printf '\xef\x82\xa0' ;;  # battery_5_bar  U+F0A0
	*) printf '\xef\x82\xa1' ;;  # battery_6_bar  U+F0A1
	esac
}

# Nerd Font MDI for the tooltip sections, matching net.sh.
IC_CHARGE='󰁹'  # md-battery      U+F0079
IC_HEALTH='󰗶'  # md-heart_pulse  U+F05F6
IC_POWER='󰉁'   # md-flash        U+F0241

# ---------------------------------------------------------------- primitives

# Batteries report either charge (µAh, needs voltage to become watts) or
# energy (µWh, already watts). Read whichever this one has and remember which.
UNIT=""
readf() { # attribute basename -> value or empty
	[ -r "$BAT/$1" ] && cat "$BAT/$1" 2>/dev/null || true
}

# now full full_design rate unit   ("" if the battery has neither set)
levels() {
	_n=$(readf charge_now) _f=$(readf charge_full) _d=$(readf charge_full_design) _r=$(readf current_now)
	if [ -n "$_n" ] && [ -n "$_f" ]; then
		printf '%s %s %s %s Ah\n' "$_n" "$_f" "${_d:-$_f}" "${_r:-0}"
		return
	fi
	_n=$(readf energy_now) _f=$(readf energy_full) _d=$(readf energy_full_design) _r=$(readf power_now)
	if [ -n "$_n" ] && [ -n "$_f" ]; then
		printf '%s %s %s %s Wh\n' "$_n" "$_f" "${_d:-$_f}" "${_r:-0}"
	fi
}

pct() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%d", (b > 0 ? a * 100 / b + 0.5 : 0) }'; }

# µ-units -> "1.25 Ah"
uh() { awk -v v="$1" -v u="$2" 'BEGIN { printf "%.2f %s", v / 1000000, u }'; }

# µA × µV -> W, or µW -> W when the battery already reports energy
watts() { # rate, voltage, unit
	awk -v r="$1" -v v="$2" -v u="$3" 'BEGIN {
		printf "%.1f W", (u == "Ah" ? r * v / 1e12 : r / 1e6)
	}'
}

# how long until empty (discharging) or full (charging), in seconds
remaining() { # now, full, rate, status
	awk -v n="$1" -v f="$2" -v r="$3" -v s="$4" 'BEGIN {
		if (r <= 0) { print -1; exit }
		print int((s == "Charging" ? f - n : n) / r * 3600)
	}'
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = "test" ]; then
	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT
	mkdir -p "$T/BAT0"
	for kv in charge_now:1254000 charge_full:2944000 charge_full_design:3550000 \
		current_now:1067000 voltage_now:10847000 cycle_count:216 \
		status:Discharging model_name:X421-35 manufacturer:OEM technology:Unknown; do
		printf '%s\n' "${kv#*:}" > "$T/BAT0/${kv%%:*}"
	done
	BAT="$T/BAT0"

	set -- $(levels)
	[ "$1 $2 $3 $4 $5" = "1254000 2944000 3550000 1067000 Ah" ] || { echo "levels wrong: $*"; exit 1; }
	[ "$(pct "$1" "$2")" = "43" ] || { echo "charge pct wrong: $(pct "$1" "$2")"; exit 1; }
	[ "$(pct "$2" "$3")" = "83" ] || { echo "health pct wrong: $(pct "$2" "$3")"; exit 1; }
	[ "$(uh "$1" Ah)" = "1.25 Ah" ] || { echo "uh wrong: $(uh "$1" Ah)"; exit 1; }
	[ "$(watts "$4" 10847000 Ah)" = "11.6 W" ] || { echo "watts wrong: $(watts "$4" 10847000 Ah)"; exit 1; }
	[ "$(hdur "$(remaining "$1" "$2" "$4" Discharging)")" = "1h 10m" ] || { echo "remaining wrong"; exit 1; }
	[ "$(hdur "$(remaining "$1" "$2" "$4" Charging)")" = "1h 35m" ] || { echo "charge time wrong"; exit 1; }
	[ "$(remaining "$1" "$2" 0 Discharging)" = "-1" ] || { echo "idle rate should be unknown"; exit 1; }

	# the level icon has to walk the whole table without falling off either end
	[ "$(ic_bat 0)" = "$(printf '\xee\xaf\x9c')" ] || { echo "empty battery icon wrong"; exit 1; }
	[ "$(ic_bat 100)" = "$(printf '\xef\x82\xa1')" ] || { echo "full battery icon wrong"; exit 1; }
	[ "$(ic_bat 50)" = "$(printf '\xef\x82\x9e')" ] || { echo "half battery icon wrong"; exit 1; }
	[ "$(ic_bat 43)" != "$(ic_bat 90)" ] || { echo "level icon must vary with charge"; exit 1; }

	# energy-reporting batteries take the other branch
	rm "$T/BAT0"/charge_* "$T/BAT0"/current_now
	printf '%s\n' 30000000 > "$T/BAT0/energy_now"
	printf '%s\n' 50000000 > "$T/BAT0/energy_full"
	printf '%s\n' 8000000 > "$T/BAT0/power_now"
	set -- $(levels)
	[ "$1 $2 $3 $4 $5" = "30000000 50000000 50000000 8000000 Wh" ] || { echo "energy levels wrong: $*"; exit 1; }
	[ "$(watts "$4" 0 Wh)" = "8.0 W" ] || { echo "energy watts wrong"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- module

[ -n "$BAT" ] || BAT=$(set -- /sys/class/power_supply/BAT*; printf '%s' "$1")
[ -n "$AC" ] || AC=$(set -- /sys/class/power_supply/A[CD]*; printf '%s' "$1")

if [ ! -d "$BAT" ] || [ -z "$(levels)" ]; then
	printf '{"text":"","class":"absent","tooltip":"No battery"}\n'
	exit 0
fi

set -- $(levels)
NOW=$1 FULL=$2 DESIGN=$3 RATE=$4 UNIT=$5
STATUS=$(readf status)
VOLT=$(readf voltage_now)
CYCLES=$(readf cycle_count)
MODEL=$(readf model_name)
VENDOR=$(readf manufacturer)
TECH=$(readf technology)
ONLINE=$([ -r "$AC/online" ] && cat "$AC/online" || echo 0)

CHARGE=$(readf capacity)
[ -n "$CHARGE" ] || CHARGE=$(pct "$NOW" "$FULL")
HEALTH=$(pct "$FULL" "$DESIGN")

case "$STATUS" in
Charging | "Not charging") ICON=$(ic_chg) ;;
*) ICON=$(ic_bat "$CHARGE") ;;
esac

CLASS=discharging
case "$STATUS" in Charging) CLASS=charging ;; Full) CLASS=full ;; esac
[ "$CHARGE" -le 20 ] && [ "$CLASS" != charging ] && CLASS=critical

TEXT="$(barico "$ICON") ${CHARGE}%"

SECS=$(remaining "$NOW" "$FULL" "$RATE" "$STATUS")

TIP=$(
	title "Battery${MODEL:+ · $MODEL}"
	rule

	sect "$IC_CHARGE" "Charge"
	row "$(printf '%3s%%  %s' "$CHARGE" "$(bar "$CHARGE" "$(grade $((100 - CHARGE)) 70 80)")")"
	row "$(uh "$NOW" "$UNIT") of $(uh "$FULL" "$UNIT")  ·  $STATUS"
	if [ "$SECS" -gt 0 ]; then
		case "$STATUS" in
		Charging) dim "$(hdur "$SECS") until full" ;;
		*) dim "$(hdur "$SECS") left at this rate" ;;
		esac
	elif [ "$ONLINE" = 1 ]; then
		dim "on AC, not drawing"
	fi

	# Wear is the interesting direction: 100% health is good, so grade the
	# complement or a healthy battery would read red.
	sect "$IC_HEALTH" "Health"
	row "$(printf '%3s%%  %s' "$HEALTH" "$(bar "$HEALTH" "$(grade $((100 - HEALTH)) 20 35)")")"
	row "$(uh "$FULL" "$UNIT") of $(uh "$DESIGN" "$UNIT") when new  ·  $((100 - HEALTH))% worn"
	[ -n "$CYCLES" ] && [ "$CYCLES" != 0 ] && dim "$CYCLES charge cycles"
	[ -n "$VENDOR$TECH" ] && dim "$(printf '%s %s' "${VENDOR:-?}" "${TECH:-?}" | esc)"

	sect "$IC_POWER" "Power"
	if [ "$RATE" -gt 0 ]; then
		row "$(watts "$RATE" "${VOLT:-0}" "$UNIT") draw"
	else
		row "idle"
	fi
	[ -n "$VOLT" ] && dim "$(awk -v v="$VOLT" 'BEGIN { printf "%.2f V", v / 1000000 }')$([ "$UNIT" = Ah ] && awk -v r="$RATE" 'BEGIN { printf "  ·  %.2f A", r / 1000000 }')"
)

emit "$CLASS" "$TEXT" "$TIP"
