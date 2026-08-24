#!/bin/sh
# Keep-awake toggle for ironbar's inhibit module (on-click): starts/stops
# mango-keepawake.service, which holds a `systemd-inhibit
# --what=idle:handle-lid-switch --mode=block` inhibitor. Replaces ironbar's
# native `inhibit` module (genconfig.rs's old inhibit_module() doc comment
# has the full story) — that module's portal call is a silent no-op on this
# session, so the toggle used to change a glyph and inhibit nothing.
set -u

UNIT="mango-keepawake.service"

case "${1:-}" in
--toggle)
	if systemctl --user is-active --quiet "$UNIT"; then
		systemctl --user stop "$UNIT" || notify-send -a mango-bard "Keep-awake" "Failed to stop"
	else
		OUT=$(systemctl --user start "$UNIT" 2>&1) || notify-send -a mango-bard "Keep-awake" "Failed to start: $OUT"
	fi
	mango-bard refresh keepawake 2>/dev/null
	;;
*)
	echo "usage: keepawake.sh --toggle" >&2
	exit 1
	;;
esac
