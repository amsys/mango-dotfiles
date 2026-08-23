#!/usr/bin/env bash
# rofi prompt: capture an intruding thought to the parking lot without
# breaking focus — FOCUS.md §2.2/§5.4's "note it in 5 seconds, then back to
# the task". Also bumps the running pomodoro's distraction tally.
#
#   focus-note.sh --launch   spawn the prompt (bound to SUPER+SHIFT+N)
#   focus-note.sh test       assert the line-building helper
set -euo pipefail

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/mango-focus"
LOT="$DATA_DIR/parking-lot.md"

# ISO-8601 timestamp, a tab, then the note text — one line per entry, same
# shape focus-review.sh's `open_loops()` reads back.
note_line() { printf '%s\t%s\n' "$(printf '%(%Y-%m-%dT%H:%M:%S)T' -1)" "$1"; }

if [ "${1:-}" = "test" ]; then
	L=$(TZ=UTC note_line "buy milk")
	[[ "$L" == *$'\t'"buy milk" ]] || { echo "note_line wrong: $L"; exit 1; }
	echo "ok"
	exit 0
fi

[ "${1:-}" = "--launch" ] || exit 0

NOTE=$(rofi -dmenu -p "Parking lot" -theme ~/.config/rofi/focus-input.rasi -l 0)
[ -n "$NOTE" ] || exit 0
mkdir -p "$DATA_DIR"
note_line "$NOTE" >> "$LOT"
mango-bard pomo note -q
