#!/usr/bin/env bash
# Weekly focus review — FOCUS.md §2.3/§5.6: a 7-day summary from
# mango-bard's log.tsv as the rofi header, then the open parking-lot loops
# (FOCUS.md §2.2) below it; selecting one finishes it (moved to done.md) —
# per the loop-audit protocol, "finish it, schedule it, or consciously kill
# it", this covers "finish". Leaving a loop unselected leaves it open for
# next week.
#
#   focus-review.sh --launch   spawn the rofi review (bound to SUPER+SHIFT+R)
#   focus-review.sh test       assert the summary/open-loop helpers
set -euo pipefail

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/mango-focus"
LOG="$DATA_DIR/log.tsv"
LOT="$DATA_DIR/parking-lot.md"
DONE="$DATA_DIR/done.md"

# log.tsv columns (pomo.rs's `log_phase`): date, start, end, phase, minutes,
# task, distractions. Only "work" rows count toward the pomodoro tally —
# a break row's distraction count is whatever was live when the break
# started, not a count of its own. No break-skip figure: nothing in
# pomo.rs's control surface can skip a break outright (mute and reset are
# not a skip), so there is nothing true to report there.
summary() { # log-file, cutoff-date (YYYY-MM-DD, inclusive)
	awk -F'\t' -v cutoff="$2" '
	{
		if ($1 >= cutoff && $4 == "work") { n++; d += $7 }
	}
	END {
		if (n == 0) { print "no focus blocks in the last 7 days"; exit }
		printf "%d pomodoros • %.1f distractions/pomodoro\n", n, d / n
	}' "$1"
}

# Parking-lot lines whose text (everything after the timestamp tab) hasn't
# been moved to done.md yet.
open_loops() { # parking-lot file, done file
	[ -r "$1" ] || return 0
	local done_text=""
	[ -r "$2" ] && done_text=$(cut -f2- "$2")
	while IFS=$'\t' read -r ts text; do
		[ -n "$text" ] || continue
		grep -qxF "$text" <<< "$done_text" || printf '%s\t%s\n' "$ts" "$text"
	done < "$1"
}

if [ "${1:-}" = "test" ]; then
	T=$(mktemp)
	cat > "$T" <<-'EOF'
	2026-08-15	09:00	09:25	work	25		1
	2026-08-15	09:25	09:30	short-break	5		0
	2026-08-22	10:00	10:25	work	25	plan trip	3
	EOF
	OUT=$(summary "$T" 2026-08-16)
	[ "$OUT" = "1 pomodoros • 3.0 distractions/pomodoro" ] \
		|| { echo "summary wrong: $OUT"; exit 1; }
	[ "$(summary "$T" 2026-09-01)" = "no focus blocks in the last 7 days" ] \
		|| { echo "summary should report the empty case"; exit 1; }
	rm -f "$T"

	L=$(mktemp); D=$(mktemp)
	printf '2026-08-20T09:00:00\tbuy milk\n2026-08-20T09:05:00\tcall bank\n' > "$L"
	printf '2026-08-20T09:00:00\tbuy milk\n' > "$D"
	OUT=$(open_loops "$L" "$D")
	[ "$OUT" = "$(printf '2026-08-20T09:05:00\tcall bank')" ] \
		|| { echo "open_loops wrong: $OUT"; exit 1; }
	rm -f "$L" "$D"

	echo "ok"
	exit 0
fi

[ "${1:-}" = "--launch" ] || exit 0

CUTOFF=$(printf '%(%Y-%m-%d)T' "$(($(printf '%(%s)T' -1) - 7 * 86400))")
HEADER=$(summary "$LOG" "$CUTOFF" 2> /dev/null || echo "no focus log yet")

SELECTED=$(open_loops "$LOT" "$DONE" | cut -f2- \
	| rofi -dmenu -p "Open loops" -mesg "$HEADER" -theme ~/.config/rofi/focus-input.rasi)
[ -n "$SELECTED" ] || exit 0

mkdir -p "$DATA_DIR"
printf '%s\t%s\n' "$(printf '%(%Y-%m-%dT%H:%M:%S)T' -1)" "$SELECTED" >> "$DONE"
