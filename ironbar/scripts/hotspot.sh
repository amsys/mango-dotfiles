#!/bin/sh
# Virtual Wi-Fi hotspot for ironbar's hotspot module, plus a rofi menu that
# starts it through the current VPN (protected) or direct (unprotected).
#
# The uplink is the laptop's default route, so a VPN is picked with net.sh's
# NetworkManager picker (`net.sh --sec-click`), never with a list of its own.
# The menu opens again once after that picker closes and the VPN state changed.
# A protected hotspot adds a FORWARD reject rule (mango-hotspot PROTECT=1), so
# its clients lose the internet when the VPN drops instead of going direct.
# See docs/features/hotspot-vpn/SPEC.md.
#
# Four modes:
#   --status  custom/hotspot exec — up/down + tooltip
#   --menu    on-click            — state-dependent start/pick/stop menu
#   --toggle  on-click-middle     — toggle without opening the menu
#   test      self-check (no root, no network) — asserts freq_to_chan()
#             and short_label()
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

notify() { notify-send -a mango-bard "Hotspot" "$1"; }

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

# "NordVPN (al134.nordvpn.com)" -> "NordVPN (al134)"; other names unchanged.
short_label() { printf '%s' "$1" | sed 's/\.nordvpn\.com)$/)/'; }

# Name of the active VPN connection that carries the default route, shortened;
# prints nothing when the uplink is direct. A VPN is an active NM connection of
# type wireguard, vpn or tun whose DEVICE is the route's dev. NAME comes last
# in the nmcli line because it may hold ':' (escaped as '\:' in -t output).
uplink_label() {
	ul_dev=$(ip -o route get 1.1.1.1 2>/dev/null |
		awk '{for (i = 1; i < NF; i++) if ($i == "dev") {print $(i + 1); exit}}')
	[ -n "$ul_dev" ] || return 0
	nmcli -t -f TYPE,DEVICE,NAME connection show --active 2>/dev/null |
		while IFS=: read -r ul_type ul_cdev ul_name; do
			case "$ul_type" in wireguard | vpn | tun) ;; *) continue ;; esac
			[ "$ul_cdev" = "$ul_dev" ] || continue
			short_label "$(printf '%s' "$ul_name" | sed 's/\\:/:/g')"
			break
		done
}

is_protected() { [ -e "${RUN_DIR:-/run/mango-hotspot}/protected" ]; }

client_count() {
	f="/run/mango-hotspot/dnsmasq.leases"
	[ -r "$f" ] && wc -l <"$f" || echo 0
}

# do_up [0|1]: 1 = protected (clients never go out direct), default 0.
do_up() {
	ensure_creds
	BC=$(uplink_bandchan 2>&1) || { notify "$BC"; return 1; }
	BAND=$(printf '%s\n' "$BC" | cut -d' ' -f1)
	CHANNEL=$(printf '%s\n' "$BC" | cut -d' ' -f2)
	OUT=$(printf 'SSID=%s\nPSK=%s\nBAND=%s\nCHANNEL=%s\nPROTECT=%s\n' "$SSID" "$PSK" "$BAND" "$CHANNEL" "${1:-0}" |
		sudo -n /usr/local/bin/mango-hotspot up 2>&1) || { notify "Failed to start: $OUT"; return 1; }
}

do_down() {
	OUT=$(sudo -n /usr/local/bin/mango-hotspot down 2>&1) || notify "Failed to stop: $OUT"
}

toggle() {
	if is_up; then
		do_down
	elif [ -n "$(uplink_label)" ]; then
		do_up 1
	else
		do_up 0
	fi
	# Refresh on the actual state-change edge only — NOT from a trap on
	# every exit. hotspot.rs has no watcher of its own (IRONBAR.md T6b —
	# there is no stable inode to inotify across up/down cycles), so this
	# poke, scoped to `toggle()` (reached only from `--toggle`/`--menu`,
	# never from `--status`), is the only edge that tells the daemon
	# anything changed. An EXIT-trap version of this poke used to also
	# pkill waybar unconditionally on every `--status` run, which waybar's
	# own signal handler answered by re-running `--status` — an unbounded
	# self-feeding loop, live and measured before this poke was scoped down
	# to `toggle()` alone. waybar is gone now; the scoping lesson stays.
	mango-bard refresh hotspot 2>/dev/null
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	[ "$(freq_to_chan 2452)" = "bg 9" ] || { echo "freq_to_chan 2.4GHz wrong"; exit 1; }
	[ "$(freq_to_chan 5180)" = "a 36" ] || { echo "freq_to_chan 5GHz wrong"; exit 1; }
	freq_to_chan 5500 >/dev/null 2>&1 && { echo "freq_to_chan should refuse a DFS channel"; exit 1; }
	[ "$(short_label 'NordVPN (al134.nordvpn.com)')" = "NordVPN (al134)" ] || { echo "short_label NordVPN wrong"; exit 1; }
	[ "$(short_label 'ProtonVPN CH-12')" = "ProtonVPN CH-12" ] || { echo "short_label ProtonVPN wrong"; exit 1; }
	[ -z "$(short_label '')" ] || { echo "short_label empty wrong"; exit 1; }
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
	[ -n "$UL" ] && CLASS=vpn
	UL="${UL:-direct}"
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
UL=$(uplink_label)
ULM=$(printf '%s' "$UL" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
ROWS=""
ACTS=""
# add_row "text" action — rofi returns the row index; ACTS is the same list.
add_row() {
	ROWS="${ROWS:+$ROWS
}$1"
	ACTS="${ACTS:+$ACTS
}$2"
}
PWROW="Show password"

if is_up; then
	CHAN=$(iw dev "$IFACE" info 2>/dev/null | awk '/channel/ {print $2; exit}')
	N=$(client_count)
	STATUS="● On  ·  $SSID  ·  ch $CHAN  ·  $N clients"
	if is_protected && [ -n "$UL" ]; then
		add_row "$STATUS" none
		add_row "Via  $ULM  ✓ protected        ›" pick
	elif is_protected; then
		add_row "⚠ VPN down — clients have no internet" none
		add_row "Reconnect a VPN                  ›" pick
	else
		add_row "$STATUS" none
		add_row "Via  ${ULM:-direct}  ⚠ not protected" none
		add_row "Pick a VPN and protect           ›" pickprot
	fi
	add_row "Turn hotspot off" off
else
	if [ -n "$UL" ]; then
		add_row "▶ Start hotspot via $ULM" up1
		add_row "Via  $ULM  ✓ protected        ›" pick
	else
		add_row "Pick a VPN, then start           ›" pick
		add_row "Start unprotected (direct)" up0
	fi
fi
add_row "$PWROW" pw

IDX=$(printf '%s\n' "$ROWS" | rofi -dmenu -format i -markup-rows -p "Hotspot" \
	-theme-str 'window { width: 480px; }')
RC=$?
[ "$RC" = 0 ] || exit 0
case "$IDX" in '' | *[!0-9]*) exit 0 ;; esac

ACT=$(printf '%s\n' "$ACTS" | sed -n "$((IDX + 1))p")

case "$ACT" in
up1)
	do_up 1
	mango-bard refresh hotspot 2>/dev/null
	;;
up0)
	do_up 0
	mango-bard refresh hotspot 2>/dev/null
	;;
off)
	do_down
	mango-bard refresh hotspot 2>/dev/null
	;;
pick | pickprot)
	# Wait for the shared VPN picker, then reopen this menu once if the VPN
	# state changed. The reopened menu never reopens on its own: only a new
	# press on a "›" row runs the picker again.
	"$(dirname "$0")/net.sh" --sec-click
	NEW=$(uplink_label)
	if [ "$ACT" = pickprot ] && [ -n "$NEW" ]; then
		OUT=$(sudo -n /usr/local/bin/mango-hotspot protect 2>&1) || notify "Failed to protect: $OUT"
		mango-bard refresh hotspot 2>/dev/null
	fi
	[ "$NEW" = "$UL" ] && [ "$ACT" != pickprot ] && exit 0
	exec "$0" --menu
	;;
pw) notify "SSID: $SSID  ·  Password: $PSK" ;;
esac
