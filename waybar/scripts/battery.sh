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
RAPL_PKG="${MANGO_RAPL_PKG_DIR:-/sys/class/powercap/intel-rapl:0}"
RAPL_UNC="${MANGO_RAPL_UNC_DIR:-/sys/class/powercap/intel-rapl:0:1}"
UPOWER_DIR="${MANGO_UPOWER_DIR:-/var/lib/upower}"
BACKLIGHT_DIR="${MANGO_BACKLIGHT_DIR:-}"
RAPL_STATE="${XDG_RUNTIME_DIR:-/tmp}/waybar-battery-rapl"

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

# Material Symbols Rounded energy_savings_leaf, U+EC1A — the eco-mode marker
# on the bar icon. Colour is inline pango, not a CSS class: waybar's `.class`
# tints the whole widget, and `.critical` already needs the NN% text red at
# <=20%, so the two would fight over one colour if this were a class instead.
# #a6da95 matches @ok in matugen/templates/waybar/style.css — deliberately not
# matugen-derived, same reasoning as the netsec lock: a mode signal has to
# read as green regardless of the wallpaper.
ic_leaf_green() { printf '<span foreground="#a6da95">\xee\xb0\x9a</span>'; }

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
watts_num() { # rate, voltage, unit -> bare number, no unit suffix
	awk -v r="$1" -v v="$2" -v u="$3" 'BEGIN {
		printf "%.1f", (u == "Ah" ? r * v / 1e12 : r / 1e6)
	}'
}
watts() { printf '%s W' "$(watts_num "$1" "$2" "$3")"; }

# ------------------------------------------------------- RAPL power attribution

readf_path() { [ -r "$1" ] && cat "$1" 2>/dev/null || true; }
rapl_readable() { [ -r "$RAPL_PKG/energy_uj" ] && [ -r "$RAPL_UNC/energy_uj" ]; }

# energy_uj deltas wrap around max_energy_range_uj rather than going negative
# forever, same idea as cpu.sh's deltas() clamping across a suspend/resume
# counter reset — a wrap here must read as a small positive draw, not a
# negative or huge nonsense wattage.
rapl_watts() { # prev_uj, prev_epoch_ns, cur_uj, cur_epoch_ns, max_range_uj -> W
	awk -v pu="$1" -v pn="$2" -v cu="$3" -v cn="$4" -v mr="$5" 'BEGIN {
		d = cu - pu
		if (d < 0) { if (mr > 0) d += mr; else d = 0 }
		if (d < 0) d = 0
		dt = (cn - pn) / 1000000000
		printf "%.1f", (dt > 0 ? d / dt / 1000000 : 0)
	}'
}

# part's share of whole, as a 0-100 percentage for bar()
sharepct() { # part, whole
	awk -v p="$1" -v w="$2" 'BEGIN {
		v = (w > 0 ? p * 100 / w : 0); if (v > 100) v = 100; if (v < 0) v = 0
		printf "%.0f", v
	}'
}

# upower logs one history-rate-<model>-<serial>.dat per power-supply device
# (battery, mouse, keyboard, ...); matching on the model name (spaces ->
# underscores, same as upower's own filenames) picks the battery's out of the
# pile without needing the serial.
power_hist_file() {
	_m=$(printf '%s' "$MODEL" | tr ' ' '_')
	set -- "$UPOWER_DIR/history-rate-$_m"-*.dat
	[ -e "$1" ] && printf '%s' "$1"
}

# tab-separated "epoch watts state" lines -> "min max avg  b1 b2 ... bN",
# bucketed oldest-to-newest for heatbar(). Only "discharging" samples inside
# the window count — on AC the file still fills with "charging"/"unknown"
# rows, and mixing those in would understate the real draw. Empty buckets
# carry the previous bucket's value forward rather than reading as 0 W, or
# a quiet stretch would look like an idle trough that never happened.
power_stats() { # file, now, window=3600, buckets=24
	awk -v now="$2" -v win="${3:-3600}" -v nb="${4:-24}" -F'\t' '
		$3 == "discharging" && $1 >= now - win {
			n++; s += $2
			if (n == 1 || $2 < mn) mn = $2
			if (n == 1 || $2 > mx) mx = $2
			b = int((now - $1) * nb / win); if (b >= nb) b = nb - 1; if (b < 0) b = 0
			bs[b] += $2; bn[b]++
		}
		END {
			if (!n) exit 1
			printf "%.1f %.1f %.1f", mn, mx, s / n
			# buckets come out normalized 0-100 within this window own range,
			# not raw watts — heatbar glyph/colour picks assume that scale,
			# same as every other module per-cell series.
			span = mx - mn
			last = 50
			for (i = nb - 1; i >= 0; i--) {
				if (bn[i]) { v = bs[i] / bn[i]; last = (span > 0 ? (v - mn) * 100 / span : 50) }
				printf " %.0f", last
			}
			printf "\n"
		}' "$1"
}

# backlight brightness as a percentage, or nothing on a machine with no panel
# (desktop, external-monitor-only laptop lid closed permanently, etc.)
backlight_pct() {
	_bl="$BACKLIGHT_DIR"
	[ -n "$_bl" ] || _bl=$(set -- /sys/class/backlight/*; [ -d "$1" ] && printf '%s' "$1")
	[ -n "$_bl" ] || return 1
	_br=$(readf_path "$_bl/brightness") _mx=$(readf_path "$_bl/max_brightness")
	[ -n "$_br" ] && [ -n "$_mx" ] && [ "$_mx" -gt 0 ] || return 1
	pct "$_br" "$_mx"
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

	# --- RAPL delta arithmetic, including a counter wrap ---
	# 100000 uj over 1s (1e9 ns) with plenty of headroom below max_range -> 0.1 W
	[ "$(rapl_watts 100000 0 200000 1000000000 999999999)" = "0.1" ] \
		|| { echo "rapl_watts wrong: $(rapl_watts 100000 0 200000 1000000000 999999999)"; exit 1; }
	# counter passed max_energy_range_uj and wrapped back near 0: 900000 -> 100000
	# with a 1000000 range is really +200000 uj, not -800000
	[ "$(rapl_watts 900000 0 100000 1000000000 1000000)" = "0.2" ] \
		|| { echo "rapl_watts wrap wrong: $(rapl_watts 900000 0 100000 1000000000 1000000)"; exit 1; }
	# counter went backwards and the range is unknown (0) -> clamp to 0, never negative
	[ "$(rapl_watts 900000 0 100000 1000000000 0)" = "0.0" ] \
		|| { echo "rapl_watts unknown-range clamp wrong: $(rapl_watts 900000 0 100000 1000000000 0)"; exit 1; }

	[ "$(sharepct 5 20)" = "25" ] || { echo "sharepct wrong: $(sharepct 5 20)"; exit 1; }
	[ "$(sharepct 30 20)" = "100" ] || { echo "sharepct should clamp above 100%"; exit 1; }
	[ "$(sharepct 5 0)" = "0" ] || { echo "sharepct with no total should read 0, not divide by zero"; exit 1; }

	# --- upower history file selection + parsing ---
	mkdir -p "$T/upower"
	: > "$T/upower/history-rate-generic_id.dat"              # peripheral noise
	: > "$T/upower/history-rate-ThinkPad_Keyboard-aa:bb.dat" # peripheral noise
	UPOWER_DIR="$T/upower" MODEL="X421-35"
	HF="$T/upower/history-rate-X421-35-42-123456789.dat"
	: > "$HF"
	[ "$(power_hist_file)" = "$HF" ] || { echo "power_hist_file picked the wrong file: $(power_hist_file)"; exit 1; }

	NOWT=1700000000
	{
		printf '%s\t%s\t%s\n' "$((NOWT - 3500))" 10 discharging
		printf '%s\t%s\t%s\n' "$((NOWT - 3000))" 15 discharging
		printf '%s\t%s\t%s\n' "$((NOWT - 1800))" 5 charging     # wrong state, excluded
		printf '%s\t%s\t%s\n' "$((NOWT - 1200))" 20 discharging
		printf '%s\t%s\t%s\n' "$((NOWT - 100))" 50 unknown      # wrong state, excluded
		printf '%s\t%s\t%s\n' "$((NOWT - 7200))" 999 discharging # outside the 1h window, excluded
	} > "$HF"
	PS=$(power_stats "$HF" "$NOWT" 3600 24)
	set -- $PS
	# hand-calculated over the three in-window discharging samples: 10, 15, 20
	[ "$1 $2 $3" = "10.0 20.0 15.0" ] || { echo "power_stats min/max/avg wrong: $1 $2 $3"; exit 1; }

	: > "$T/empty.dat"
	power_stats "$T/empty.dat" "$NOWT" 3600 24 > /dev/null 2>&1 \
		&& { echo "power_stats should fail (nothing to show) on an empty/all-filtered file"; exit 1; }

	# --- on-AC branch: "Where it goes" gates on Discharging, not just RAPL being readable ---
	mkdir -p "$T/rapl0" "$T/rapl1"
	printf '1000\n' > "$T/rapl0/energy_uj"
	printf '500\n' > "$T/rapl1/energy_uj"
	RAPL_PKG="$T/rapl0" RAPL_UNC="$T/rapl1"
	rapl_readable || { echo "rapl_readable should be true when both energy_uj files exist"; exit 1; }
	# the module only draws "Where it goes" when STATUS = Discharging, even though
	# RAPL itself is readable the whole time on AC too
	STATUS_FOR_TEST=Charging
	[ "$STATUS_FOR_TEST" = Discharging ] && rapl_readable \
		&& { echo "Where it goes must not show while Charging"; exit 1; }
	STATUS_FOR_TEST=Discharging
	{ [ "$STATUS_FOR_TEST" = Discharging ] && rapl_readable; } \
		|| { echo "Where it goes should show when Discharging and RAPL is readable"; exit 1; }

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

MODE=$(power_mode)
WEAK=""
[ -f "${XDG_RUNTIME_DIR:-/tmp}/mango-powermode.weak" ] && WEAK=1
LEAF=""
[ "$MODE" = eco ] && LEAF=" $(barico "$(ic_leaf_green)")"

TEXT="$(barico "$ICON") ${CHARGE}%${LEAF}"

SECS=$(remaining "$NOW" "$FULL" "$RATE" "$STATUS")

# --- RAPL power attribution ---
# Only meaningful while discharging (nothing to subtract a package watt from
# on AC), and only once system/rapl/install.sh has unlocked energy_uj — before
# that this whole block is a no-op and the tooltip just doesn't gain the
# "Where it goes" section, no error anywhere.
PKG_W="" UNC_W="" RESID_W="" BL="" TOTAL_W=""
if [ "$STATUS" = Discharging ] && rapl_readable; then
	PKG_UJ=$(readf_path "$RAPL_PKG/energy_uj")
	UNC_UJ=$(readf_path "$RAPL_UNC/energy_uj")
	NOW_NS=$(date +%s%N)
	if [ -n "$PKG_UJ" ] && [ -n "$UNC_UJ" ]; then
		if [ -f "$RAPL_STATE" ] && read -r P_NS P_PKG P_UNC < "$RAPL_STATE"; then
			MRP=$(readf_path "$RAPL_PKG/max_energy_range_uj"); MRP=${MRP:-0}
			MRU=$(readf_path "$RAPL_UNC/max_energy_range_uj"); MRU=${MRU:-0}
			PKG_W=$(rapl_watts "$P_PKG" "$P_NS" "$PKG_UJ" "$NOW_NS" "$MRP")
			UNC_W=$(rapl_watts "$P_UNC" "$P_NS" "$UNC_UJ" "$NOW_NS" "$MRU")
			TOTAL_W=$(watts_num "$RATE" "${VOLT:-0}" "$UNIT")
			RESID_W=$(awk -v t="$TOTAL_W" -v p="$PKG_W" 'BEGIN { r = t - p; printf "%.1f", (r < 0 ? 0 : r) }')
			BL=$(backlight_pct)
		fi
		# always refresh the sample, even on the first run with nothing to diff
		# against yet — otherwise the section stays empty forever, not just once
		printf '%s %s %s\n' "$NOW_NS" "$PKG_UJ" "$UNC_UJ" > "$RAPL_STATE"
	fi
fi


# Wear, DIMM-style hardware info and the power-history graph move slowly, so
# the tooltip rebuilds on a much slower clock (60s) than the 15s poll — but
# the RAPL sampling above always runs every poll regardless: it is a
# two-sample delta against $RAPL_STATE, and skipping a sample would bias the
# next wattage reading across whatever gap the cache introduced.
TIP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-battery-tip"
# $MODE folded in so a toggle repaints instantly, not just on the next
# minute's bucket — CLASS alone doesn't change on a mode flip.
TIP_KEY="$CLASS-$MODE-$(tip_bucket 60)"
if tip_stale "$TIP_CACHE" "$TIP_KEY"; then
TIP=$(
	title "Battery${MODEL:+ · $MODEL}"
	rule 44

	kv Charge "$(bar "$CHARGE" "$(grade $((100 - CHARGE)) 70 80)") $(mono "$(printf '%3s%%' "$CHARGE")")"
	CDET="$(uh "$NOW" "$UNIT") / $(uh "$FULL" "$UNIT") · $STATUS"
	if [ "$SECS" -gt 0 ]; then
		case "$STATUS" in
		Charging) CDET="$CDET · $(hdur "$SECS") until full" ;;
		*) CDET="$CDET · $(hdur "$SECS") left" ;;
		esac
	elif [ "$ONLINE" = 1 ]; then
		CDET="$CDET · on AC, not drawing"
	fi
	kvsub "$CDET"

	# Wear is the interesting direction: 100% health is good, so grade the
	# complement or a healthy battery would read red.
	kv Health "$(bar "$HEALTH" "$(grade $((100 - HEALTH)) 20 35)") $(mono "$(printf '%3s%%' "$HEALTH")")"
	HDET="$((100 - HEALTH))% worn"
	[ -n "$CYCLES" ] && [ "$CYCLES" != 0 ] && HDET="$HDET · $CYCLES cycles"
	kvsub "$HDET"
	[ -n "$VENDOR$TECH" ] && kvsub "$(printf '%s %s' "${VENDOR:-?}" "${TECH:-?}" | esc)"

	if [ "$RATE" -gt 0 ]; then
		PDET="$(watts "$RATE" "${VOLT:-0}" "$UNIT") draw"
	else
		PDET="idle"
	fi
	[ -n "$VOLT" ] && PDET="$PDET · $(awk -v v="$VOLT" 'BEGIN { printf "%.2f V", v / 1000000 }')$([ "$UNIT" = Ah ] && awk -v r="$RATE" 'BEGIN { printf " · %.2f A", r / 1000000 }')"
	kv Power "$PDET"
	# world-readable upower history — no RAPL/system/rapl/install.sh needed for
	# this part. Silently omitted on AC (nothing "discharging" to filter to) or
	# if the file is missing/unparseable; the draw-now row above still stands.
	HF=$(power_hist_file)
	if [ -n "$HF" ]; then
		PS=$(power_stats "$HF" "$(date +%s)" 3600 24 2> /dev/null)
		if [ -n "$PS" ]; then
			set -- $PS
			PMIN=$1 PMAX=$2 PAVG=$3
			shift 3
			kvsub "$(heatbar "$*" 70 90)  ${PMIN}–${PMAX} W over 1h, ${PAVG} W avg"
		fi
	fi

	# Needs system/rapl/install.sh to have unlocked energy_uj, a Discharging
	# status (nothing to subtract a package watt from on AC), and a second
	# sample to diff against — all three fold into PKG_W being non-empty.
	if [ -n "$PKG_W" ]; then
		kv Where "$(bar "$(sharepct "$PKG_W" "$TOTAL_W")" "$C_GOOD" 10) CPU $PKG_W W"
		kvsub "$(bar "$(sharepct "$UNC_W" "$TOTAL_W")" "$C_GOOD" 10) GPU $UNC_W W"
		kvsub "$(bar "$(sharepct "$RESID_W" "$TOTAL_W")" "$C_GOOD" 10) rest $RESID_W W$([ -n "$BL" ] && printf ' (backlight %s%%)' "$BL")"
	fi

	if [ -n "$WEAK" ]; then
		kv Mode "$(bad "eco") · weak charger"
	elif [ -f "${XDG_RUNTIME_DIR:-/tmp}/mango-powermode.manual" ]; then
		kv Mode "$MODE · manual"
	else
		kv Mode "$MODE · $([ "$ONLINE" = 1 ] && echo "on AC" || echo "on battery")"
	fi

	dim "click to toggle mode  ·  right-click for a powertop report"
)
	tip_save "$TIP_CACHE" "$TIP_KEY" "$TIP"
else
	TIP=$(tip_load "$TIP_CACHE")
fi

emit "$CLASS" "$TEXT" "$TIP"
