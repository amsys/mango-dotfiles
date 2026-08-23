#!/usr/bin/env bash
# Screenshot to a file (and the clipboard), active monitor or a slurp
# selection. One wrapper so the save directory lives in exactly one place
# instead of being duplicated across binds.
#
# Usage:
#   screenshot.sh monitor   # active output, no selection step
#   screenshot.sh all       # every output in one image
#   screenshot.sh window    # click a visible window (slurp -r over mmsg rects)
#   screenshot.sh region    # slurp selection
#   screenshot.sh test      # self-check
set -u

# mango's env= parser does a literal setenv with no ~ or $HOME expansion (see
# README's "Machine-specific values"), so a leading ~/ has to be expanded here
# or MANGO_SCREENSHOT_DIR=~/Shots would create a directory literally named ~.
DIR="${MANGO_SCREENSHOT_DIR:-${XDG_PICTURES_DIR:-$HOME/Pictures}/Screenshots}"
# literal ~/ prefix check, not a shell expansion
# shellcheck disable=SC2088
case "$DIR" in
"~/"*) DIR="$HOME/${DIR#"~/"}" ;;
esac

notify() { notify-send -a screenshot -u low "$@"; }

active_monitor() { mmsg get all-monitors | jq -r '.monitors[]|select(.active)|.name'; }

if [ "${1:-}" = "test" ]; then
	# The two things that actually break silently: a filename template that
	# produces a colon (unplayable on anything mounted FAT/exFAT), and the
	# active-monitor query coming back empty.
	name="Screenshot_$(date '+%Y-%m-%d_%H.%M.%S').png"
	case "$name" in
	*:*) echo "fail: timestamp contains a colon"; exit 1 ;;
	esac
	for bin in grim slurp wl-copy jq mmsg; do
		command -v "$bin" >/dev/null || { echo "fail: $bin not installed"; exit 1; }
	done
	mon=$(active_monitor)
	[ -n "$mon" ] || { echo "fail: no active monitor from mmsg"; exit 1; }
	echo "ok"
	exit 0
fi

mon=""
case "${1:-}" in
monitor)
	mon=$(active_monitor)
	[ -n "$mon" ] || { notify "Screenshot failed" "no active monitor"; exit 1; }
	geom=""
	;;
all)
	# grim with no -o/-g captures every output in one image.
	geom=""
	;;
window)
	# Visible-window rectangles from the compositor, picked with one click —
	# slurp -r restricts the selection to the given rects. Same
	# cancel-quietly rule as region below.
	rects=$(mmsg get all-clients |
		jq -r '.clients[]|select(.is_visible)|"\(.x),\(.y) \(.width)x\(.height)"')
	[ -n "$rects" ] || { notify "Screenshot failed" "no visible windows"; exit 1; }
	geom=$(printf '%s\n' "$rects" | slurp -r) || exit 0
	[ -n "$geom" ] || exit 0
	;;
region)
	# slurp writes nothing and exits non-zero when cancelled with Escape. Bail
	# quietly rather than capturing the whole screen by accident.
	geom=$(slurp) || exit 0
	[ -n "$geom" ] || exit 0
	;;
*)
	echo "usage: screenshot.sh monitor|all|window|region|test" >&2
	exit 1
	;;
esac

mkdir -p "$DIR"
file="$DIR/Screenshot_$(date '+%Y-%m-%d_%H.%M.%S').png"

if [ -n "$geom" ]; then
	grim -g "$geom" "$file"
elif [ -n "$mon" ]; then
	grim -o "$mon" "$file"
else
	grim "$file"
fi

wl-copy <"$file"
notify "Screenshot saved" "$(basename "$file")"
