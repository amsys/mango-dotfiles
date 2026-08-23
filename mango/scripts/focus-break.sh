#!/usr/bin/env bash
# Full-screen phase-boundary overlay — FOCUS.md §4.3: a blocking, screen-
# filling banner instead of a corner toast you learn to ignore. Water
# reminder rides the break bell itself (§1: sip at every break), so there is
# no separate water timer.
#
#   focus-break.sh "<phase label>" <minutes>   spawned by mango-bard's
#                                               pomo.rs at every phase
#                                               boundary — never launched
#                                               directly.
set -euo pipefail

LABEL="${1:?usage: focus-break.sh <phase-label> <minutes>}"
MINUTES="${2:?usage: focus-break.sh <phase-label> <minutes>}"

MSG="$LABEL — $MINUTES min"
case "$LABEL" in
*[Bb]reak*) MSG="$MSG — sip water" ;;
esac

# `-e` (message mode): no candidates, no filtering, just the text — Esc or
# Enter dismisses. focus.rasi sizes the window full-screen.
rofi -e "$MSG" -theme ~/.config/rofi/focus.rasi
