#!/bin/sh
# Ethernet adapter toggle for ironbar's eth module (on-click).
set -u

DEV="${MANGO_ETH_DEV:-eno2}"
STATE=$(nmcli -t -f DEVICE,STATE device status | awk -F: -v d="$DEV" '$1==d{print $2; exit}')

case "$STATE" in
connected* | connecting*)
	nmcli device disconnect "$DEV" || notify-send -a mango-bard "Ethernet" "Failed to disable $DEV"
	;;
*)
	OUT=$(nmcli device connect "$DEV" 2>&1) || notify-send -a mango-bard "Ethernet" "Failed to enable $DEV: $OUT"
	;;
esac
