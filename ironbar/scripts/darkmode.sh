#!/usr/bin/env bash
# Dark/light toggle for ironbar (design ported from end-4/dots-hyprland). State lives in gsettings,
# same place switchwall.sh reads/writes it. --toggle re-runs the full color pipeline
# via switchwall.sh --noswitch (matugen + reload) rather than duplicating it here.
set -u

SWITCHWALL="$HOME/.config/mango/scripts/switchwall.sh"

is_dark() {
	[[ "$(gsettings get org.gnome.desktop.interface color-scheme 2>/dev/null)" != *"prefer-light"* ]]
}

if [[ "${1:-}" == "--toggle" ]]; then
	if is_dark; then
		setsid "$SWITCHWALL" --mode light --noswitch &
	else
		setsid "$SWITCHWALL" --mode dark --noswitch &
	fi
	exit 0
fi

# Icon shows the action, not the state: light_mode glyph while dark
# (click to go light), dark_mode glyph while light (click to go dark).
if is_dark; then
	printf ''  # light_mode
else
	printf ''  # dark_mode
fi
