#!/bin/sh
# Remote-access toggle for ironbar's remote module (on-click): wayvnc
# (screen) + kdeconnectd (input/clipboard/files), started/stopped together.
# Reachable only on wg_hetzner and the mango hotspot — see
# system/remote/install.sh for the ufw rules and system/remote/README.md
# for why. Neither unit is enabled at login; this is the only way they run.
set -u

UNITS="wayvnc.service kdeconnectd.service"

case "${1:-}" in
--toggle)
	if systemctl --user is-active --quiet wayvnc.service; then
		systemctl --user stop $UNITS || notify-send -a mango-bard "Remote access" "Failed to stop"
	else
		OUT=$(systemctl --user start $UNITS 2>&1) || notify-send -a mango-bard "Remote access" "Failed to start: $OUT"
	fi
	mango-bard refresh remote 2>/dev/null
	;;
*)
	echo "usage: remote.sh --toggle" >&2
	exit 1
	;;
esac
