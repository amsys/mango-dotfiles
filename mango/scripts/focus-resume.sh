#!/usr/bin/env bash
# Ready-to-Resume prompt — FOCUS.md §2.1: one line, where you are + the exact
# next action, written the moment a named work block ends. Externalizes the
# open loop so attention residue drops, instead of carrying it silently into
# the break.
#
#   focus-resume.sh "<task>"   spawned by mango-bard's pomo.rs (Stage B
#                               "alert") at the work bell, only when a task
#                               was named — never launched directly.
set -euo pipefail

TASK="${1:?usage: focus-resume.sh <task>}"

LINE=$(rofi -dmenu -p "Resume \"$TASK\" — next:" -theme ~/.config/rofi/focus-input.rasi -l 0)
[ -n "$LINE" ] || exit 0

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/mango-focus"
mkdir -p "$DATA_DIR"
printf '%s\t%s\t%s\n' "$(printf '%(%Y-%m-%dT%H:%M:%S)T' -1)" "$TASK" "$LINE" \
	>> "$DATA_DIR/resume.md"
mango-bard pomo resume "$TASK — $LINE" -q
