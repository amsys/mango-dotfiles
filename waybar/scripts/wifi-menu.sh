#!/bin/sh
# Wi-Fi quick-connect menu for waybar's `network` module (on-click).
# ponytail: menu-driven, one nmcli call per action — no daemon, no caching.
set -u

DEV="${MANGO_WIFI_DEV:-wlo1}"
CACHE="$HOME/.cache/wifi-menu-location"
SCAN_FLAG="${XDG_RUNTIME_DIR:-/tmp}/wifi-scan"
notify() { notify-send -a waybar "Wi-Fi" "$1"; }

# The rescan below blocks for a few seconds with nothing on screen. net-watch.sh
# already spins for every NM state change, but a rescan changes no device state
# so NM tells it nothing — ask by hand. The flag file carries our pid, which is
# how net.sh knows to hide custom/wifi for the duration (the spinner replaces
# the arc, it never sits beside it) and how it recovers if we are SIGKILLed.
spin_start() {
	printf '%s\n' "$$" >"$SCAN_FLAG"
	pkill -USR1 -f net-watch.sh 2>/dev/null
	pkill -RTMIN+12 waybar 2>/dev/null
	return 0
}

spin_stop() {
	rm -f "$SCAN_FLAG"
	pkill -USR2 -f net-watch.sh 2>/dev/null
	return 0
}

# Joining, disconnecting or disabling the radio all change the signal readout
# *and* the security verdict, so nudge net.sh's two modules (10 = netsec,
# 12 = wifi/eth) rather than making them wait out their 5s interval. On the
# trap so every early exit path — cancel included — is covered.
refresh_bar() {
	pkill -RTMIN+10 waybar 2>/dev/null
	pkill -RTMIN+12 waybar 2>/dev/null
	return 0
}
trap 'spin_stop; refresh_bar' EXIT INT TERM

# "<Country> <City>" for the profile name. Only resolvable once we're online,
# so this runs *after* the join; the cache covers captive portals and offline.
# ponytail: ipinfo.io — the system timezone and the AP beacon country IE both
# lie while travelling.
geo() {
	json=$(curl -s --max-time 4 https://ipinfo.io/json 2>/dev/null)
	cc=$(printf '%s' "$json" | sed -n 's/.*"country": *"\([^"]*\)".*/\1/p')
	city=$(printf '%s' "$json" | sed -n 's/.*"city": *"\([^"]*\)".*/\1/p')
	country=$(awk -F'\t' -v c="$cc" 'c != "" && $1 == c {print $2; exit}' /usr/share/zoneinfo/iso3166.tab)
	if [ -n "$country" ]; then
		mkdir -p "${CACHE%/*}"
		printf '%s%s\n' "$country" "${city:+ $city}" | tee "$CACHE"
	else
		cat "$CACHE" 2>/dev/null
	fi
}

if [ "$(nmcli radio wifi)" = "disabled" ]; then
	nmcli radio wifi on || notify "Failed to enable Wi-Fi"
	exit 0
fi

spin_start

# "<ssid>\t<uuid>" for every saved wireless profile. Matched on the SSID, never
# on the profile name — names carry a "<Country> <City> - " prefix and so stop
# matching the SSID as soon as a network is saved.
UUIDS=$(nmcli -t -f UUID,TYPE connection show | awk -F: '$2 == "802-11-wireless" {print $1}')
SAVED=""
[ -n "$UUIDS" ] && SAVED=$(nmcli -t -f 802-11-wireless.ssid,connection.uuid connection show $UUIDS |
	grep -v '^$' | sed 's/\\:/:/g; s/^802-11-wireless\.ssid://; s/^connection\.uuid://' |
	paste - - | awk -F'\t' '!seen[$1]++')

saved_uuid() { printf '%s\n' "$SAVED" | awk -F'\t' -v s="$1" '$1 == s {print $2; exit}'; }

# One row per SSID: "in-use \t signal \t band-rank \t band \t security \t ssid".
# Sorted by connected first, then signal, then band — so a duplicate SSID
# collapses to its strongest AP and 5/6 GHz wins any tie with 2.4 GHz.
# nmcli backslash-escapes colons *inside* field values only, so rejoining
# fields 5..NF with ":" restores an SSID that contained one.
SCAN=$(nmcli -t -f IN-USE,SIGNAL,FREQ,SECURITY,SSID device wifi list --rescan yes ifname "$DEV" |
	awk -F: '{
		ssid = $5
		for (i = 6; i <= NF; i++) ssid = ssid ":" $i
		gsub(/\\:/, ":", ssid)
		if (ssid == "") next
		mhz = $3 + 0
		band = mhz >= 5925 ? "6G" : (mhz >= 4900 ? "5G" : "2.4G")
		rank = mhz >= 4900 ? 1 : 0
		printf "%s\t%s\t%s\t%s\t%s\t%s\n", $1, $2, rank, band, $4, ssid
	}' | sort -t "$(printf '\t')" -k1,1r -k2,2rn -k3,3rn |
	awk -F'\t' '!seen[$6]++')

# Labels are built in awk, not a `read` loop: tab is an IFS *whitespace*
# character, so `read` collapses the empty SECURITY field of an open network
# and shifts every column after it.
# The metrics column lives in <tt> because Google Sans Flex has no ▮/▯ and pads
# proportionally — monospace is what actually lines the SSIDs up.
MENU=$(printf '%s\n@@@\n%s\n' "$SAVED" "$SCAN" | awk -F'\t' '
	function esc(s) { gsub(/&/, "\\&amp;", s); gsub(/</, "\\&lt;", s); gsub(/>/, "\\&gt;", s); return s }
	$0 == "@@@" { scan = 1; next }
	!scan { if ($1 != "") saved[$1] = 1; next }
	$6 != "" {
		sig = $2 + 0
		gauge = sig >= 75 ? "▮▮▮▮" : (sig >= 50 ? "▮▮▮▯" : (sig >= 25 ? "▮▮▯▯" : "▮▯▯▯"))
		# nmcli reports every accepted suite ("WPA1 WPA2", "WPA2 WPA3") — collapse
		# to one token so a mixed-mode AP still shows it accepts the weak one.
		sec = $5
		if (sec == "" || sec == "--") {
			sec = "open"
		} else if (sec ~ /WPA/) {
			v = ""
			if (sec ~ /WPA1/) v = "1"
			if (sec ~ /WPA2/) v = v (v ? "/" : "") "2"
			if (sec ~ /WPA3/) v = v (v ? "/" : "") "3"
			sec = "WPA" v (sec ~ /802\.1X/ ? "+E" : "")
		}
		mark = ($1 == "*" ? " (connected)" : "") ($6 in saved ? " [saved]" : "")
		printf "<tt><span alpha=\"65%%\">%s %3d%%  %-4s %-8s</span></tt> %s<span alpha=\"55%%\">%s</span>\n",
			gauge, sig, $4, sec, esc($6), mark
	}')

RULE='<span alpha="30%">────────────────────</span>'
ACTIONS="  Disconnect
⏻  Disable Wi-Fi"

N=0
[ -n "$SCAN" ] && N=$(printf '%s\n' "$SCAN" | wc -l)
if [ "$N" -gt 0 ]; then
	LIST=$(printf '%s\n%s\n%s' "$MENU" "$RULE" "$ACTIONS")
else
	LIST=$(printf '%s\n%s' "$RULE" "$ACTIONS")
fi

spin_stop

# Selection comes back as a row index, so the decorated labels above never have
# to be parsed back into an SSID. Shift+Return needs unbinding from
# kb-accept-alt before kb-custom-1 can take it; kb-custom-1 exits 10.
IDX=$(printf '%s\n' "$LIST" |
	rofi -dmenu -format i -markup-rows -p "Wi-Fi" \
		-mesg "Shift+Enter to name the connection yourself" \
		-theme-str 'window { width: 720px; } listview { spacing: 5px; } element { padding: 9px 8px; }' \
		-kb-accept-alt "" -kb-custom-1 "Shift+Return")
RC=$?
[ "$RC" = 0 ] || [ "$RC" = 10 ] || exit 0
case "$IDX" in '' | *[!0-9]*) exit 0 ;; esac

if [ "$IDX" -ge "$N" ]; then
	case $((IDX - N)) in
	1) nmcli device disconnect "$DEV" || notify "Disconnect failed" ;;
	2) nmcli radio wifi off || notify "Failed to disable Wi-Fi" ;;
	esac
	exit 0
fi

ROW=$(printf '%s\n' "$SCAN" | sed -n "$((IDX + 1))p")
SSID=$(printf '%s\n' "$ROW" | cut -f6)
SEC=$(printf '%s\n' "$ROW" | cut -f5)
UUID=$(saved_uuid "$SSID")
WAS_SAVED=$UUID

if [ -n "$UUID" ]; then
	OUT=$(nmcli connection up uuid "$UUID" 2>&1) || { notify "Failed to join $SSID: $OUT"; exit 1; }
elif [ -z "$SEC" ] || [ "$SEC" = "--" ]; then
	OUT=$(nmcli device wifi connect "$SSID" ifname "$DEV" 2>&1) || { notify "Failed to join $SSID: $OUT"; exit 1; }
else
	PW=$(rofi -dmenu -password -p "Password for $SSID")
	[ -z "$PW" ] && exit 0
	OUT=$(nmcli device wifi connect "$SSID" password "$PW" ifname "$DEV" 2>&1) || { notify "Failed to join $SSID: $OUT"; exit 1; }
fi

# Name a freshly created profile "<Country> <City> - <SSID>"; Shift+Enter opens
# that name for editing (for an already-saved network, its current name).
[ -z "$UUID" ] && UUID=$(nmcli -t -f UUID,DEVICE connection show --active |
	awk -F: -v d="$DEV" '$2 == d {print $1; exit}')
[ -z "$UUID" ] && exit 0

if [ "$RC" = 10 ] || [ -z "$WAS_SAVED" ]; then
	NAME=$(nmcli -g connection.id connection show uuid "$UUID" 2>/dev/null)
	if [ -z "$WAS_SAVED" ]; then
		PREFIX=$(geo)
		NAME="${PREFIX:+$PREFIX - }$SSID"
	fi
	[ "$RC" = 10 ] && NAME=$(printf '%s\n' "$NAME" | rofi -dmenu -format f -filter "$NAME" -p "Name")
	[ -n "$NAME" ] && nmcli connection modify uuid "$UUID" connection.id "$NAME"
fi
