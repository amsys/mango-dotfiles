#!/bin/sh
# Virtual Wi-Fi hotspot for waybar's custom/hotspot, plus a rofi picker for
# the uplink (direct or NordVPN).
#
# Four modes:
#   --status  custom/hotspot exec — up/down + tooltip
#   --menu    on-click            — toggle + uplink picker
#   --toggle  on-click-middle     — toggle without opening the menu
#   test      self-check (no root, no network) — asserts freq_to_chan()
#
# Why this drives hostapd/dnsmasq directly instead of NetworkManager's
# `shared` AP mode: this box's chip (Intel AX201) fails to create a plain
# `type __ap` vif while wlo1 stays connected — confirmed live, reproducible
# ENFILE from mac80211. A `type __p2pgo` vif takes a different firmware path
# that does work, but NM has no connection type that drives a P2P-GO
# interface like a normal AP, so mango-hotspot (root helper) runs hostapd on
# it directly. See mango-hotspot's own header for the full story.
#
# The hotspot's channel is locked to whatever wlo1 is on — the driver's
# interface-combination table caps this chip at one channel shared across
# every vif — so it is read fresh from `iw dev wlo1 link` on every toggle,
# never stored.
set -u

DEV="${MANGO_WIFI_DEV:-wlo1}"
IFACE="${MANGO_HS_IFACE:-p2p0}"
CACHE="$HOME/.cache/mango-hotspot.conf"

. "$(dirname "$0")/tooltip.sh"

notify() { notify-send -a waybar "Hotspot" "$1"; }

# freq(MHz) -> "band channel" on stdout, exit 1 for anything this chip's AP
# path can't do: DFS 5GHz (no radar detection in P2P-GO/AP mode) or a band
# outside the IR-CONCURRENT ranges reported by `iw reg get`'s self-managed
# phy0 entry.
freq_to_chan() {
	f=$1
	if [ "$f" -ge 2400 ] && [ "$f" -le 2500 ]; then
		printf 'bg %s\n' "$(((f - 2407) / 5))"
		return 0
	fi
	case "$f" in
	5180 | 5200 | 5220 | 5240 | 5745 | 5765 | 5785 | 5805 | 5825)
		printf 'a %s\n' "$(((f - 5000) / 5))"
		return 0
		;;
	esac
	return 1
}

# "band channel" for the hotspot, or empty + a message on stderr if the
# uplink is somewhere this chip can't put an AP.
uplink_bandchan() {
	freq=$(iw dev "$DEV" link 2>/dev/null | awk '/^\tfreq:/ {print int($2)}')
	if [ -z "$freq" ]; then
		printf 'bg 6\n' # wlo1 down/disconnected — pick a sane default
		return 0
	fi
	if ! freq_to_chan "$freq"; then
		echo "uplink is on a DFS channel — move $DEV to a non-DFS channel first" >&2
		return 1
	fi
}

ensure_creds() {
	if [ ! -r "$CACHE" ]; then
		mkdir -p "$(dirname "$CACHE")"
		PSK=$(openssl rand -hex 8)
		printf 'SSID=mango-hotspot\nPSK=%s\n' "$PSK" >"$CACHE"
		chmod 600 "$CACHE"
	fi
	# shellcheck disable=SC1090  # $CACHE is a fixed, script-defined path
	. "$CACHE"
}

is_up() { ip link show "$IFACE" >/dev/null 2>&1; }

uplink_label() {
	if ip link show nordlynx >/dev/null 2>&1; then
		printf 'NordVPN'
	else
		printf 'direct'
	fi
}

client_count() {
	f="/run/mango-hotspot/dnsmasq.leases"
	[ -r "$f" ] && wc -l <"$f" || echo 0
}

do_up() {
	ensure_creds
	BC=$(uplink_bandchan 2>&1) || { notify "$BC"; return 1; }
	BAND=$(printf '%s\n' "$BC" | cut -d' ' -f1)
	CHANNEL=$(printf '%s\n' "$BC" | cut -d' ' -f2)
	OUT=$(printf 'SSID=%s\nPSK=%s\nBAND=%s\nCHANNEL=%s\n' "$SSID" "$PSK" "$BAND" "$CHANNEL" |
		sudo -n /usr/local/bin/mango-hotspot up 2>&1) || { notify "Failed to start: $OUT"; return 1; }
}

do_down() {
	OUT=$(sudo -n /usr/local/bin/mango-hotspot down 2>&1) || notify "Failed to stop: $OUT"
}

toggle() {
	if is_up; then do_down; else do_up; fi
	# Refresh both bars on the actual state-change edge only — NOT from a
	# trap on every exit. custom/hotspot's own exec is `--status`
	# (config.jsonc:353), and waybar's module is itself signal-driven
	# ("signal": 16, config.jsonc:356): an EXIT trap that pkilled
	# unconditionally used to fire on every `--status` run too, which
	# waybar's own signal handler then answered by re-running `--status`,
	# which fired the trap again — an unbounded self-feeding loop (measured
	# live: waybar spinning ~25-28% CPU from it, and once mango-bard's
	# refresh poke rode the same trap, flooding mango-bard's control socket
	# too). Scoping both pokes to `toggle()` — reached only from `--toggle`
	# and `--menu`'s own dispatch, never from `--status` — makes them
	# edge-triggered for real, matching what IRONBAR.md T6b decision D2
	# actually needs from this.
	pkill -RTMIN+16 waybar 2>/dev/null
	mango-bard refresh hotspot 2>/dev/null
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	[ "$(freq_to_chan 2452)" = "bg 9" ] || { echo "freq_to_chan 2.4GHz wrong"; exit 1; }
	[ "$(freq_to_chan 5180)" = "a 36" ] || { echo "freq_to_chan 5GHz wrong"; exit 1; }
	freq_to_chan 5500 >/dev/null 2>&1 && { echo "freq_to_chan should refuse a DFS channel"; exit 1; }
	echo ok
	exit 0
fi

# ---------------------------------------------------------------- status

if [ "${1:-}" = --status ]; then
	if ! is_up; then
		printf '{"text":""}\n'
		exit 0
	fi
	ensure_creds
	UL=$(uplink_label)
	CHAN=$(iw dev "$IFACE" info 2>/dev/null | awk '/channel/ {print $2; exit}')
	N=$(client_count)
	TIP="$(title "Hotspot")
$(rule 30)
$(sect "" "$SSID  ·  ch $CHAN")
$(kv Password "$PSK")
$(kv Uplink "$UL")
$(kv Clients "$N connected")"
	CLASS=active
	[ "$UL" = NordVPN ] && CLASS=vpn
	# wifi_tethering, Material Symbols Rounded — codepoint carried over from the
	# classic Material Icons PUA mapping like net.sh's icons, but unverified on
	# this box; check it renders once the bar picks this module up and swap the
	# byte string below if it shows as tofu.
	emit "$CLASS" "$(barico "$(printf '\xee\x87\x99')")" "$TIP"
	exit 0
fi

# ---------------------------------------------------------------- toggle

if [ "${1:-}" = --toggle ]; then
	toggle
	exit 0
fi

# ---------------------------------------------------------------- menu

[ "${1:-}" = --menu ] || { echo "usage: hotspot.sh --status|--menu|--toggle|test" >&2; exit 1; }

ensure_creds
STATE="off"
is_up && STATE="on"
CHAN=""
is_up && CHAN=" · ch $(iw dev "$IFACE" info 2>/dev/null | awk '/channel/ {print $2; exit}')"
UL=$(uplink_label)

RULE='<span alpha="30%">────────────────────</span>'
ROW1="Hotspot: $STATE  ·  $SSID$CHAN"
ROW2="Uplink: direct"
ROW3="Uplink: NordVPN — $UL"
ROW4="Show password"

LIST=$(printf '%s\n%s\n%s\n%s\n%s' "$ROW1" "$RULE" "$ROW2" "$ROW3" "$ROW4")

IDX=$(printf '%s\n' "$LIST" | rofi -dmenu -format i -markup-rows -p "Hotspot" \
	-theme-str 'window { width: 480px; }')
RC=$?
[ "$RC" = 0 ] || exit 0
case "$IDX" in '' | *[!0-9]*) exit 0 ;; esac

case "$IDX" in
0) toggle ;;
2) nordvpn disconnect >/dev/null 2>&1 || notify "Already direct" ;;
3)
	# Already on NordVPN — nothing to pick, the row exists to show/confirm state.
	[ "$UL" = NordVPN ] && exit 0
	# nordvpn column-formats this list (multiple names per line, padded) when it
	# detects a terminal on its inherited stdout, which it still does even
	# piped straight into rofi — flatten on whitespace so each name is its own
	# rofi row, or a multi-name line comes back as one invalid string. -i makes
	# rofi's filter case-insensitive regardless of the user's rofi defaults.
	COUNTRY=$(nordvpn countries 2>/dev/null | tr -s ' \t' '\n' | grep . | rofi -dmenu -i -p "Country")
	[ -n "$COUNTRY" ] && {
		OUT=$(nordvpn connect "$COUNTRY" 2>&1) || notify "NordVPN connect failed: $OUT"
	}
	;;
4) notify "SSID: $SSID  ·  Password: $PSK" ;;
esac
