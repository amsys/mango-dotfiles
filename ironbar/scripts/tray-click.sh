#!/bin/sh
# ironbar tray on_click_left — genconfig.rs's tray_module(). Replaces the
# default SNI activate for every tray item (ironbar substitutes {address}
# per item, confirmed against the vendored tray source). A plain SNI
# Activate on an already-open window only hides it, so instead this always
# pulls the window to the tag you're on — never jumps your view away to
# wherever the window already lives.
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

	clients='{"clients":[{"pid":1297,"id":9,"monitor":"DP-1","is_minimized":false}]}'
	line=$(echo "$clients" | jq -r --argjson p 1297 \
		'first(.clients[] | select(.pid == $p) | "\(.id)\t\(.monitor)\t\(.is_minimized)") // empty')
	[ "$line" = "$(printf '9\tDP-1\tfalse')" ] || {
		echo "client selector wrong: $line"
		exit 1
	}

	monitors='{"monitors":[{"name":"DP-1","active":false},{"name":"eDP-1","active":true,"active_tags":[2]}]}'
	mline=$(echo "$monitors" | jq -r \
		'first(.monitors[] | select(.active) | "\(.name)\t\(.active_tags[0])") // empty')
	[ "$mline" = "$(printf 'eDP-1\t2')" ] || {
		echo "monitor selector wrong: $mline"
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

CLIENT=""
if [ -n "$PID" ]; then
	CLIENT=$(mmsg get all-clients 2>/dev/null |
		jq -r --argjson p "$PID" \
			'first(.clients[] | select(.pid == $p) | "\(.id)\t\(.monitor)\t\(.is_minimized)") // empty')
fi

if [ -z "$CLIENT" ]; then
	# No mapped surface for this tray item (hidden to tray) — SNI Activate
	# makes the app re-map its own window, which mango then places on the
	# current tag on its own.
	busctl --user call "$BUS" "$OBJ" org.kde.StatusNotifierItem Activate ii 0 0 2>/dev/null
	exit 0
fi

ID=$(echo "$CLIENT" | cut -f1)
CLIENT_MON=$(echo "$CLIENT" | cut -f2)
MINIMIZED=$(echo "$CLIENT" | cut -f3)

# A minimized client (e.g. KeePassXC's named-scratchpad hide) only comes
# back through client_active() -> show_hide_client(), which re-tags it to
# its OLD tag — tag/tagmon do not un-minimize on their own. Restore it
# first, then pull it below; the brief old-tag flash is the tradeoff for
# not open-coding mango's minimize-restore logic here.
if [ "$MINIMIZED" = true ]; then
	mmsg dispatch focusid client,"$ID" 2>/dev/null
fi

CUR=$(mmsg get all-monitors 2>/dev/null |
	jq -r 'first(.monitors[] | select(.active) | "\(.name)\t\(.active_tags[0])") // empty')

if [ -z "$CUR" ]; then
	# Couldn't read where "here" is — fall back to the old jump-to-window
	# behaviour rather than leaving the click dead.
	mmsg dispatch focusid client,"$ID" 2>/dev/null
	exit 0
fi

CUR_MON=$(echo "$CUR" | cut -f1)
CUR_TAG=$(echo "$CUR" | cut -f2)

# tagmon, not tagcrossmon, for the cross-monitor case: tagcrossmon only
# calls tag_client() when the target monitor is already selected, so a
# window on the OTHER output would keep its old monitor. tagmon,<mon>,0
# always calls setmon(), which retags the client onto the target
# monitor's current tagset.
if [ "$CLIENT_MON" = "$CUR_MON" ]; then
	mmsg dispatch tag,"$CUR_TAG",0 client,"$ID" 2>/dev/null
else
	mmsg dispatch tagmon,"$CUR_MON",0 client,"$ID" 2>/dev/null
fi
