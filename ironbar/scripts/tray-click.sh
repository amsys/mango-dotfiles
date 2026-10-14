#!/bin/sh
# ironbar tray on_click_left — genconfig.rs's tray_module(). Replaces the
# default SNI Activate for every tray item (ironbar substitutes {address}
# per item, confirmed against the vendored tray source). When the app has a
# mapped window, clicking the tray icon calls focusid on that window, which
# makes mango un-minimize it and jump the view to the window's tag. A window
# hidden to tray has no client, so SNI Activate makes the app re-map it on
# the current tag.
#
# $1 = the item's D-Bus address, "bus/object/path" (ironbar's own {address}
# substitution — e.g. ":1.50128/StatusNotifierItem").
set -u

if [ "${1:-}" = test ]; then
	addr=":1.50128/StatusNotifierItem"
	bus="${addr%%/*}"
	obj="/${addr#*/}"
	[ "$bus" = ":1.50128" ] || {
		echo "bus split wrong: $bus"
		exit 1
	}
	[ "$obj" = "/StatusNotifierItem" ] || {
		echo "object-path split wrong: $obj"
		exit 1
	}

	clients='{"clients":[{"pid":1297,"id":9,"monitor":"DP-1"}]}'
	line=$(echo "$clients" | jq -r --argjson p 1297 \
		'first(.clients[] | select(.pid == $p) | "\(.id)") // empty')
	[ "$line" = "9" ] || {
		echo "client selector wrong: $line"
		exit 1
	}

	echo ok
	exit 0
fi

ADDR="${1:?usage: tray-click.sh <address>}"
BUS="${ADDR%%/*}"
OBJ="/${ADDR#*/}"

PID=$(busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
	org.freedesktop.DBus GetConnectionUnixProcessID s "$BUS" 2>/dev/null | awk '{print $2}')

ID=""
if [ -n "$PID" ]; then
	ID=$(mmsg get all-clients 2>/dev/null |
		jq -r --argjson p "$PID" \
			'first(.clients[] | select(.pid == $p) | "\(.id)") // empty')
fi

if [ -n "$ID" ]; then
	# Mapped client: focusid un-minimizes it and calls view_in_mon on the
	# client's tags, so mango jumps the view to the window's tag.
	mmsg dispatch focusid client,"$ID" 2>/dev/null
	exit 0
fi

# No mapped surface for this tray item (hidden to tray) — SNI Activate
# makes the app re-map its own window, which mango then places on the
# current tag on its own.
busctl --user call "$BUS" "$OBJ" org.kde.StatusNotifierItem Activate ii 0 0 2>/dev/null
exit 0
