#!/usr/bin/env bash
# rofi prompt: name the single task for this pomodoro, then start a work
# block with it — FOCUS.md §1.3 "one task per pomodoro".
#
#   focus-task.sh --launch   spawn the prompt (bound to SUPER+SHIFT+T, and
#                             mango-bard pomo.rs's own "click" verb while idle)
set -euo pipefail

[ "${1:-}" = "--launch" ] || exit 0

TASK=$(rofi -dmenu -p "Focus on" -theme ~/.config/rofi/focus-input.rasi -l 0)
[ -n "$TASK" ] || exit 0
mango-bard pomo start "$TASK" -q
