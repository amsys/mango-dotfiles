#!/usr/bin/env bash
# Clock + world clocks + calendar + pomodoro, as two waybar modules.
#
# Replaces the built-in `clock` and `clock#date` pair. waybar's clock can scroll
# through timezones but cannot show several at once, and its tooltip is a strftime
# string — no meters, so no pomodoro progress either.
#
# Time and date are two modules because waybar hangs the tooltip and the clicks
# off the module, and they want different ones: the world clocks on the time,
# the month on the date.
#
# The pomodoro has no daemon: waybar polls this script every second, so the poll
# *is* the tick. State is four fields in $XDG_RUNTIME_DIR.
#
#   clock.sh             custom/clock exec — emit JSON
#   clock.sh --toggle    on-click        — start / pause / resume
#   clock.sh --reset     on-click-right  — back to idle
#   clock.sh --date      custom/date exec
#   clock.sh --calendar  custom/date on-click — focus/open the calendar app
#   clock.sh test        assert the phase, timezone, face and calendar arithmetic
#
# ponytail: the timer only advances while waybar is polling. Kill waybar
# mid-pomodoro and the phase change fires late, when it comes back.
set -u

. "$(dirname "$0")/tooltip.sh"

STATE="${XDG_RUNTIME_DIR:-/tmp}/mango-pomodoro"
SIGNAL=14  # matches "signal" in the custom/clock module

# work,short-break,long-break (minutes), pomodoros per long break
IFS=, read -r P_WORK P_SHORT P_LONG P_CYCLE <<< "${MANGO_POMODORO:-25,5,15,4}"

# Zones for the tooltip. Machine-specific, same convention as MANGO_ETH_DEV.
WORLD="${MANGO_WORLD_TZ:-America/New_York,Europe/Prague,Asia/Bangkok,Indian/Mauritius}"

# What the date click opens. Thunderbird remotes -calendar into a running
# instance (switches its tab) rather than starting a second process.
CAL_CMD="${MANGO_CALENDAR_CMD:-thunderbird -calendar}"
CAL_APPID="${MANGO_CALENDAR_APPID:-org.mozilla.Thunderbird}"

# World clocks, three to a row. The time row is enlarged, so it must be padded
# in *its own* cells: at TIME_SCALE it takes 100/TIME_SCALE the cells of the
# label row to span the same pixels. The self-check asserts the two match.
COL_W=15; COL_GAP=3
T_W=10; T_GAP=2
TIME_SCALE=150
WORLD_COLS=3

# Material Symbols Rounded for the bar, same wrapper as every other module.
IC_WORK=$''   # timer
IC_BREAK=$''  # self_improvement

# Nerd Font MDI for tooltip sections.
IC_LOCAL='󰥔'  # md-clock              U+F0954
IC_EARTH='󰇧'  # md-earth              U+F01E7
IC_POMO='󰔛'   # md-timer_outline      U+F051B

# ---------------------------------------------------------------- columns

# Centre plain text in a cell width. The padding stays outside any markup, so
# length() is never counting span tags.
ctr() { # text, width
	awk -v s="$1" -v w="$2" 'BEGIN {
		p = w - length(s); if (p < 0) p = 0
		printf "%*s%s%*s", int(p / 2), "", s, p - int(p / 2), ""
	}'
}

# ---------------------------------------------------------------- primitives

# minutes for a phase name
phase_len() { # phase
	case "$1" in
	work) echo $((P_WORK * 60)) ;;
	short) echo $((P_SHORT * 60)) ;;
	long) echo $((P_LONG * 60)) ;;
	*) echo 0 ;;
	esac
}

# which phase follows, given how many work blocks are already done
next_phase() { # phase, completed-count
	case "$1" in
	work) [ $(($2 % P_CYCLE)) -eq 0 ] && echo long || echo short ;;
	*) echo work ;;
	esac
}

phase_label() { # phase
	case "$1" in
	work) echo "Focus" ;;
	short) echo "Short break" ;;
	long) echo "Long break" ;;
	*) echo "Idle" ;;
	esac
}

# "America/New_York" -> "New York". A few zones are named after the country
# rather than the city they mean; spell those out.
tz_label() {
	case "$1" in
	Indian/Mauritius) echo "Port Louis"; return ;;
	esac
	local l=${1##*/}
	echo "${l//_/ }"
}

# The zone /etc/localtime points at, so the world list can skip repeating it.
local_zone() { realpath /etc/localtime 2> /dev/null | sed 's|.*/zoneinfo/||'; }

# +0400 vs +0100 -> "-3h", "+2h30", "" when identical
tz_offset() { # remote %z, local %z
	local r=$1 l=$2 rm lm d sign
	rm=$((10#${r:1:2} * 60 + 10#${r:3:2})); [ "${r:0:1}" = "-" ] && rm=$((-rm))
	lm=$((10#${l:1:2} * 60 + 10#${l:3:2})); [ "${l:0:1}" = "-" ] && lm=$((-lm))
	d=$((rm - lm))
	[ "$d" -eq 0 ] && { echo "same"; return; }
	sign=+; [ "$d" -lt 0 ] && { sign=-; d=$((-d)); }
	if [ $((d % 60)) -eq 0 ]; then printf '%s%dh\n' "$sign" $((d / 60))
	else printf '%s%dh%02d\n' "$sign" $((d / 60)) $((d % 60)); fi
}

# The month as a bordered table, today highlighted. `cal -m` is Monday-first
# (matching the bar's "%a, %d %b") and lays days out in fixed 3-character
# columns, so the cells are read off those columns — matching the number with a
# regex would also hit the 2 inside 12 and 21.
#
# 36 cells wide (│ + 7 × "─dd─│"), flush left with no indent: this table is the
# widest line in the tooltip, so it is what pins the tooltip's width, and the
# CSS `tooltip label` padding is then the only margin on either side. Borders
# are C_EMPTY, darker than both the day numbers and the weekday header.
#
# Each line is wrapped in one border-coloured span and the day cells nest their
# own colour inside it — one span per day rather than one per separator.
cal_grid() { # day-of-month, [month year]
	cal -m ${2:+$2} | awk -v today="$1" -v dim="$C_DIM" -v hi="$C_TITLE" \
		-v body="$C_LABEL" -v brd="$C_EMPTY" '
	function border(l, m, r,   s, i) {
		s = l
		for (i = 0; i < 7; i++) s = s "────" (i < 6 ? m : r)
		return sprintf("<span foreground=\"%s\">%s</span>", brd, s)
	}
	function cells(line, colour, bold,   out, i, c) {
		out = "│"
		for (i = 0; i < 7; i++) {
			c = substr(line, i * 3 + 1, 2)
			if (c ~ /[^ ]/)
				c = sprintf("<span foreground=\"%s\"%s>%2s</span>",
					(bold && c + 0 == today) ? hi : colour,
					(bold && c + 0 == today) ? " font_weight=\"bold\"" : "", c)
			else
				c = "  "
			out = out " " c " │"
		}
		return sprintf("<span foreground=\"%s\">%s</span>", brd, out)
	}
	NR == 1 { next }   # the month name is the tooltip title already
	NR == 2 {
		print border("╭", "┬", "╮")
		print cells($0, dim, 0)
		print border("├", "┼", "┤")
		next
	}
	/[0-9]/ { print cells($0, body, 1) }   # guard: cal can emit a trailing blank line
	END { print border("╰", "┴", "╯") }'
}

notify() { notify-send -a pomodoro -t 4000 -h string:x-canonical-private-synchronous:pomodoro "$1" "$2"; }

beep() { # sound file basename
	local f=/usr/share/sounds/freedesktop/stereo/$1.oga
	[ -r "$f" ] && command -v paplay > /dev/null 2>&1 && (paplay "$f" > /dev/null 2>&1 &)
}

# ---------------------------------------------------------------- state

# echoes: state phase until count   (state = run|pause; nothing when idle)
load() { [ -r "$STATE" ] && cat "$STATE" || true; }
save() { printf '%s %s %s %s\n' "$1" "$2" "$3" "$4" > "$STATE"; }

# Advance across any phase boundaries the poll interval jumped over, firing the
# beep and notification for the one that just ended.
tick() { # now -> "state phase until count"
	local now=$1 st ph until count len
	read -r st ph until count <<< "$(load)"
	[ -n "${st:-}" ] || return 0
	if [ "$st" = run ] && [ "$now" -ge "$until" ]; then
		[ "$ph" = work ] && count=$((count + 1))
		local nxt; nxt=$(next_phase "$ph" "$count")
		len=$(phase_len "$nxt")
		if [ "$ph" = work ]; then
			notify "Pomodoro done" "$(phase_label "$nxt") · $((len / 60)) min"
			beep complete
		else
			notify "Break over" "Focus · $P_WORK min"
			beep alarm-clock-elapsed
		fi
		ph=$nxt; until=$((now + len))
		save run "$ph" "$until" "$count"
	fi
	printf '%s %s %s %s\n' "$st" "$ph" "$until" "$count"
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = "test" ]; then
	[ "$(phase_len work)" = 1500 ] && [ "$(phase_len short)" = 300 ] && [ "$(phase_len long)" = 900 ] \
		|| { echo "phase_len wrong"; exit 1; }
	[ "$(next_phase work 1)" = short ] || { echo "1st pomodoro -> short"; exit 1; }
	[ "$(next_phase work 3)" = short ] || { echo "3rd pomodoro -> short"; exit 1; }
	[ "$(next_phase work 4)" = long ] || { echo "4th pomodoro -> long"; exit 1; }
	[ "$(next_phase work 8)" = long ] || { echo "8th pomodoro -> long"; exit 1; }
	[ "$(next_phase short 3)" = work ] && [ "$(next_phase long 4)" = work ] || { echo "break -> work"; exit 1; }

	[ "$(tz_offset +0400 +0400)" = same ] || { echo "same-zone wrong"; exit 1; }
	[ "$(tz_offset +0100 +0400)" = "-3h" ] || { echo "west offset wrong: $(tz_offset +0100 +0400)"; exit 1; }
	[ "$(tz_offset +0900 +0400)" = "+5h" ] || { echo "east offset wrong"; exit 1; }
	[ "$(tz_offset +0545 +0400)" = "+1h45" ] || { echo "half-hour offset wrong: $(tz_offset +0545 +0400)"; exit 1; }
	[ "$(tz_offset -0500 +0400)" = "-9h" ] || { echo "negative zone wrong: $(tz_offset -0500 +0400)"; exit 1; }
	[ "$(tz_offset +0000 -0330)" = "+3h30" ] || { echo "negative local wrong: $(tz_offset +0000 -0330)"; exit 1; }

	[ "$(tz_label America/New_York)" = "New York" ] && [ "$(tz_label UTC)" = UTC ] || { echo "tz_label wrong"; exit 1; }
	[ "$(tz_label Indian/Mauritius)" = "Port Louis" ] || { echo "tz_label override wrong"; exit 1; }

	[ "$(ctr ab 6)" = "  ab  " ] && [ "$(ctr abc 6)" = " abc  " ] || { echo "ctr wrong"; exit 1; }

	# The enlarged time row and the label row under it must span the same width,
	# or the three world columns stop lining up.
	[ $(((T_W * WORLD_COLS + T_GAP * (WORLD_COLS - 1)) * TIME_SCALE)) \
		= $(((COL_W * WORLD_COLS + COL_GAP * (WORLD_COLS - 1)) * 100)) ] \
		|| { echo "world column widths do not match at ${TIME_SCALE}%"; exit 1; }

	# August 2026 starts on a Saturday, so Monday-first puts the 1st in column 6.
	c=$(cal_grid 12 "8 2026")
	[ "$(printf '%s\n' "$c" | grep -c "$C_TITLE")" = 1 ] || { echo "calendar must mark exactly one day"; exit 1; }
	printf '%s\n' "$c" | grep -q "font_weight=\"bold\">12</span>" || { echo "calendar marked the wrong day"; exit 1; }
	# the 2 in 12/21/25 must not be mistaken for the 2nd
	printf '%s\n' "$(cal_grid 2 "8 2026")" | grep -c "$C_TITLE" | grep -qx 1 \
		|| { echo "calendar highlight leaked into a two-digit day"; exit 1; }

	# The table is closed, and every line is the same 36 cells wide — that width
	# is the tooltip's width, so a short line would let something else pin it.
	printf '%s\n' "$c" | head -1 | grep -q "^<span foreground=\"$C_EMPTY\">╭" \
		|| { echo "calendar must open with a rounded top border"; exit 1; }
	printf '%s\n' "$c" | tail -1 | grep -q "╯</span>$" \
		|| { echo "calendar must close with a bottom border"; exit 1; }
	# 6 week rows in August 2026, plus top, header, separator and bottom
	[ "$(printf '%s\n' "$c" | wc -l)" = 10 ] || { echo "calendar row count wrong: $(printf '%s\n' "$c" | wc -l)"; exit 1; }
	# Counted in bytes, not characters, so the check does not depend on the locale
	# awk happened to start in: a border is 36 box glyphs at 3 bytes each (108),
	# a day row is 8 │ (24) plus 28 single-byte cells (52).
	w=$(printf '%s\n' "$c" | sed 's/<[^>]*>//g' \
		| while IFS= read -r l; do printf '%s' "$l" | wc -c; done | sort -un | tr '\n' ' ')
	[ "$w" = "52 108 " ] || { echo "calendar lines must all be 36 cells, got byte widths: $w"; exit 1; }
	# February 2026 is 5 rows and starts on a Monday — the bottom border still closes
	[ "$(cal_grid 1 "2 2026" | wc -l)" = 9 ] || { echo "short month row count wrong"; exit 1; }

	# a work phase whose deadline passed rolls into a break and banks the count
	STATE=$(mktemp); trap 'rm -f "$STATE"' EXIT
	notify() { :; }; beep() { :; }
	save run work 1000 0
	read -r st ph until count <<< "$(tick 1000)"
	[ "$ph $count" = "short 1" ] || { echo "boundary rollover wrong: $ph $count"; exit 1; }
	[ "$until" = $((1000 + 300)) ] || { echo "next deadline wrong: $until"; exit 1; }
	save run work 1000 3
	read -r st ph until count <<< "$(tick 1000)"
	[ "$ph $count" = "long 4" ] || { echo "long break rollover wrong: $ph $count"; exit 1; }
	save run work 2000 0
	read -r st ph until count <<< "$(tick 1000)"
	[ "$ph $count" = "work 0" ] || { echo "should not roll over early"; exit 1; }
	save pause work 400 2
	read -r st ph until count <<< "$(tick 99999)"
	[ "$st $ph $until $count" = "pause work 400 2" ] || { echo "paused clock must not advance"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- actions

NOW=$(printf '%(%s)T' -1)

case "${1:-}" in
--toggle)
	read -r st ph until count <<< "$(load)"
	case "${st:-idle}" in
	idle) save run work $((NOW + $(phase_len work))) 0; notify "Focus" "$P_WORK min" ;;
	run) save pause "$ph" $((until - NOW)) "$count" ;;                # until := seconds left
	pause) save run "$ph" $((NOW + until)) "$count" ;;
	esac
	pkill -RTMIN+$SIGNAL waybar 2> /dev/null
	exit 0
	;;
--reset)
	rm -f "$STATE"
	pkill -RTMIN+$SIGNAL waybar 2> /dev/null
	exit 0
	;;
--calendar)
	# `focusid` is mango's client_active(): it switches to the window's tag and
	# monitor and un-minimizes before focusing, so there is no tag maths here.
	# With nothing running there is no id to focus and mango focuses the new
	# window anyway, which is why the guard is the whole branch.
	id=$(mmsg get all-clients 2> /dev/null \
		| jq -r --arg a "$CAL_APPID" 'first(.clients[] | select(.appid == $a) | .id) // empty')
	[ -n "$id" ] && mmsg dispatch focusid client,"$id" 2> /dev/null
	setsid sh -c "$CAL_CMD" > /dev/null 2>&1 &
	exit 0
	;;
--date)
	# No rule() here: the table's own top border is the rule, and leaving one out
	# is what lets the table — not a 30-cell rule — pin the tooltip width. The
	# footer is dim() without the indent for the same reason, so the title, the
	# table and the footer all start at the same x.
	emit date "<span alpha='70%'>• $(printf '%(%a, %d %b)T' "$NOW")</span>" \
		"$(
			title "$(printf '%(%B %Y)T' "$NOW")"
			mono "<span size='115%'>$(cal_grid "$(printf '%(%-d)T' "$NOW")")</span>"
			printf '\n\n'
			printf '<span foreground="%s">click for the calendar</span>\n' "$C_DIM"
		)"
	exit 0
	;;
esac

# ---------------------------------------------------------------- module

read -r ST PH UNTIL COUNT <<< "$(tick "$NOW")"
ST=${ST:-idle}

TIME=$(printf '%(%H:%M)T' "$NOW")

# The clock keeps its two-font look with spans rather than the two CSS rules the
# built-in module pair needed. The date lives in custom/date now, so idle is
# just the time and the "• " separator moved over with it.
BIG="<span size='115%' font_weight='600'>$TIME</span>"

case "$ST" in
run)
	LEFT=$((UNTIL - NOW)); [ "$LEFT" -lt 0 ] && LEFT=0
	[ "$PH" = work ] && PICO=$IC_WORK || PICO=$IC_BREAK
	TEXT="$BIG <span alpha='70%'>• $(barico "$PICO") $(printf '%d:%02d' $((LEFT / 60)) $((LEFT % 60)))</span>"
	CLASS=$PH
	;;
pause)
	[ "$PH" = work ] && PICO=$IC_WORK || PICO=$IC_BREAK
	TEXT="$BIG <span alpha='45%'>• $(barico "$PICO") $(printf '%d:%02d' $((UNTIL / 60)) $((UNTIL % 60)))</span>"
	CLASS=paused
	;;
*)
	TEXT="$BIG"
	CLASS=idle
	;;
esac

LOCALZ=$(printf '%(%z)T' "$NOW")
LOCALDAY=$(printf '%(%j)T' "$NOW")
LOCALZONE=$(local_zone)

# Keyed on the displayed minute, not a TTL: the tooltip (world clocks,
# calendar) only changes when TIME does, so this costs zero staleness rather
# than trading it for a cheaper poll. Only while idle, though — a running or
# paused pomodoro's "left" countdown ticks every second (this script's poll
# *is* the tick, see the file header), and caching it would freeze the
# countdown for up to a minute. Idle is the common case, so this still
# collapses ~59 of every 60 polls into one cat.
TIP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-clock-tip"
if [ "$ST" = idle ] && ! tip_stale "$TIP_CACHE" "$TIME"; then
	TIP=$(tip_load "$TIP_CACHE")
else
TIP=$(
	title "$(printf '%(%A, %d %B %Y)T' "$NOW")"
	rule

	sect "$IC_LOCAL" "Local"
	row "$(mono "<span size='250%' font_weight='bold'>$TIME</span>")"
	dim "$(tz_label "$LOCALZONE") · $LOCALZ"

	sect "$IC_EARTH" "World"
	# Collect the zones first, then lay them out a rowful at a time.
	IFS=, read -ra ZONES <<< "$WORLD"
	Z_TIME=(); Z_LABEL=(); Z_NOTE=()
	for z in "${ZONES[@]}"; do
		# the local zone is already the Local section above
		[ -n "$z" ] && [ "$z" != "$LOCALZONE" ] || continue
		zz=$(TZ="$z" printf '%(%z)T' "$NOW")
		zd=$(TZ="$z" printf '%(%j)T' "$NOW")
		off=$(tz_offset "$zz" "$LOCALZ")
		[ "$off" = same ] && off="same time"
		if [ "$zd" != "$LOCALDAY" ]; then
			# %j wraps at new year, so compare the raw difference both ways
			off="$off$([ $((10#$zd - 10#$LOCALDAY)) -eq 1 ] || [ $((10#$LOCALDAY - 10#$zd)) -gt 300 ] && echo " +1d" || echo " -1d")"
		fi
		Z_TIME+=("$(TZ="$z" printf '%(%H:%M)T' "$NOW")")
		Z_LABEL+=("$(tz_label "$z")")
		Z_NOTE+=("$off")
	done

	# Three to a row; a fourth zone wraps onto the next row rather than squeezing.
	# The row() indent stays outside the enlarged span so it keeps its base width
	# and the three rows start at the same place.
	for ((i = 0; i < ${#Z_TIME[@]}; i += WORLD_COLS)); do
		times=''; labels=''; notes=''; tsep=''; sep=''
		for ((j = i; j < i + WORLD_COLS && j < ${#Z_TIME[@]}; j++)); do
			times+="$tsep$(ctr "${Z_TIME[j]}" "$T_W")"
			labels+="$sep$(ctr "${Z_LABEL[j]}" "$COL_W")"
			notes+="$sep$(ctr "${Z_NOTE[j]}" "$COL_W")"
			tsep=$(printf "%${T_GAP}s" '')
			sep=$(printf "%${COL_GAP}s" '')
		done
		row "$(mono "<span size='${TIME_SCALE}%' font_weight='bold'>$times</span>")"
		row "$(mono "<span foreground=\"$C_LABEL\">$labels</span>")"
		row "$(mono "<span foreground=\"$C_DIM\">$notes</span>")"
	done

	sect "$IC_POMO" "Pomodoro"
	case "$ST" in
	run | pause)
		len=$(phase_len "$PH")
		[ "$ST" = run ] && left=$((UNTIL - NOW)) || left=$UNTIL
		[ "$left" -lt 0 ] && left=0
		done_pct=$(((len - left) * 100 / (len > 0 ? len : 1)))
		row "$(phase_label "$PH")$([ "$ST" = pause ] && printf ' · %s' "$(warn paused)")  ·  $(hdur "$left") left"
		row "$(bar "$done_pct" "$([ "$PH" = work ] && echo "$C_GOOD" || echo "$C_TITLE")")"
		dim "$COUNT done this session  ·  ${P_WORK}/${P_SHORT} min, long break every $P_CYCLE"
		dim "click to $([ "$ST" = run ] && echo pause || echo resume) · right-click to reset"
		;;
	*)
		dim "click to start a ${P_WORK} min focus block"
		;;
	esac
)
	[ "$ST" = idle ] && tip_save "$TIP_CACHE" "$TIME" "$TIP"
fi

emit "$CLASS" "$TEXT" "$TIP"
