#!/usr/bin/env bash
# The power menu, wrapped.
#
# Three things wanted the same wrapper, so wlogout is no longer invoked
# directly anywhere:
#   - a warning when something is mid-flight, since this is the one screen with
#     a Shutdown button on it
#   - wlogout's geometry flags, which are long and were about to be duplicated
#     across three call sites
#   - the margins themselves, which are pixels, so a hardcoded pair would be
#     wrong on any other monitor
#
#   powermenu.sh        open the menu
#   powermenu.sh test   assert the margin arithmetic, no compositor needed
set -uo pipefail

# Panel size in pixels, centred. 3 columns x 2 rows of buttons fit comfortably.
IFS=x read -r PW PH <<< "${MANGO_POWERMENU_SIZE:-760x430}"

BUSY="$(dirname "$0")/busy.sh"

# margin_x margin_y for a PWxPH panel centred on a SWxSH screen
margins() { # screen_w screen_h
	local mx=$((($1 - PW) / 2)) my=$((($2 - PH) / 2))
	# A panel wider than the screen would give negative margins, which wlogout
	# feeds straight to gtk_widget_set_margin_* and GTK then clamps oddly.
	[ "$mx" -lt 0 ] && mx=0
	[ "$my" -lt 0 ] && my=0
	printf '%s %s\n' "$mx" "$my"
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	PW=760 PH=430
	[ "$(margins 1920 1080)" = "580 325" ] || { echo "1080p margins wrong: $(margins 1920 1080)"; exit 1; }
	[ "$(margins 3840 2160)" = "1540 865" ] || { echo "4k margins wrong: $(margins 3840 2160)"; exit 1; }
	# a panel bigger than the display must not produce negative margins
	[ "$(margins 640 480)" = "0 25" ] || { echo "small screen not clamped: $(margins 640 480)"; exit 1; }
	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- main

# Informs, does not block: you may well be shutting down *because* something is
# stuck. Critical urgency so mako keeps it up while the menu is open.
if REASON=$("$BUSY"); then
	notify-send -a power -u critical "Something is still running" \
		"$REASON. Shutting down now will interrupt it."
fi

# Ask the compositor rather than hardcoding this laptop's panel. Any failure
# here — no mmsg, no jq, not running under mango — falls back to 1080p, which
# only costs an off-centre menu.
read -r SW SH <<< "$(mmsg get all-monitors 2>/dev/null |
	jq -r 'first(.monitors[] | select(.active)) | "\(.width) \(.height)"' 2>/dev/null)"
[ "${SW:-0}" -gt 0 ] 2>/dev/null || { SW=1920 SH=1080; }
[ "${SH:-0}" -gt 0 ] 2>/dev/null || { SW=1920 SH=1080; }

read -r MX MY <<< "$(margins "$SW" "$SH")"

# -b is the column count (wlogout attaches with the buttons-per-row loop as the
# grid's left coordinate); -L/-R/-T/-B are pixel margins on the grid itself.
exec wlogout -p layer-shell -b 3 -L "$MX" -R "$MX" -T "$MY" -B "$MY" -c 12 -r 12
