#!/bin/sh
# ironbar tray on_click_left — genconfig.rs's tray_module(). Replaces the
# default SNI activate for every tray item at once (ironbar substitutes
# {address} per item, confirmed against the vendored tray source). Jumps to
# an already-open window before falling back to Activate — fixes "click
# hides the window instead of switching to its tag" for every tray app,
# ZapZap included, not just that one.
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
		jq -r --argjson p "$PID" 'first(.clients[] | select(.pid == $p) | .id) // empty')
fi

if [ -n "$ID" ]; then
	mmsg dispatch focusid client,"$ID" 2>/dev/null
else
	busctl --user call "$BUS" "$OBJ" org.kde.StatusNotifierItem Activate ii 0 0 2>/dev/null
fi
