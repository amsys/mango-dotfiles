#!/bin/sh
# Network security lock + detailed Wi-Fi / Ethernet indicators for waybar.
#
# Six modes:
#   --sec        custom/netsec exec  — grade the whole path, emit JSON
#   --sec-click  on-click (left)     — captive portal login, else the VPN picker
#   --sec-edit   on-click-right      — nm-connection-editor on the active tunnel
#   --wifi       custom/wifi exec    — RSSI / radio / link-quality detail
#   --eth        custom/eth exec     — link / addressing / counter detail
#   --selftest   assert classify() against canned inputs (no network access)
#
# ponytail: "is DNS encrypted" collapses to "does the resolver route through a
# tunnel". systemd-resolved is disabled on this box, so there is no DoT or
# DNSSEC status to read and no API that answers the real question. If resolved
# or dnscrypt-proxy ever comes back, resolver_dev() is where the real check goes.
set -u

WIFI_DEV="${MANGO_WIFI_DEV:-wlo1}"
ETH_DEV="${MANGO_ETH_DEV:-eno2}"

# wifi-menu.sh drops its pid here while it blocks on a rescan; see wifi_emit().
SCAN_FLAG="${XDG_RUNTIME_DIR:-/tmp}/wifi-scan"

# Vendor apps regenerate their profiles behind your back; the picker only
# offers tunnels you imported yourself. Matched against the profile name.
EXCLUDE_RE='(ProtonVPN|Nord|NordLynx|PVPN)'

# FIB probe targets — `ip route get` is a pure lookup, no packets are sent.
PROBE4=1.1.1.1
PROBE6=2606:4700:4700::1111

TAB=$(printf '\t')

# Palette, meters and the JSON emitter are shared with cpu/memory/battery/clock.
. "$(dirname "$0")/tooltip.sh"

notify() { notify-send -a waybar "Network" "$1"; }

# The icon fonts are private-use area, so every glyph is invisible in an editor
# — written as escapes and named here instead. Material Symbols Rounded for the
# lock states (matches every other bar module), the same four Nerd Font arcs
# the old `network` module used for signal strength.
ic_offline() { printf '\xee\x8b\x81'; } # cloud_off        U+E2C1
ic_portal() { printf '\xee\xa9\xb7'; }  # login            U+EA77
ic_open() { printf '\xef\x80\xbf'; }    # no_encryption    U+F03F
ic_exposed() { printf '\xee\xa2\x98'; } # lock_open        U+E898
ic_lock() { printf '\xee\xa2\x99'; }    # lock             U+E899
ic_conflict() { printf '\xef\x86\x84'; } # alt_route       U+F184
ic_wifioff() { printf '\xee\x99\x88'; } # signal_wifi_off  U+E648
ic_eth() { printf '\xee\xac\xaf'; }     # lan              U+EB2F
ic_ethoff() { printf '\xee\x85\xaf'; }  # cable_off        U+E16F
arc() { # 0..100 -> one of the four signal arcs
	case "$1" in
	100 | 9? | 8? | 7[5-9]) printf '\xee\x98\xbe' ;;  # U+E63E
	7? | 6? | 5?) printf '\xee\xaf\xa1' ;;            # U+EBE1
	4? | 3? | 2[5-9]) printf '\xee\xaf\x96' ;;        # U+EBD6
	*) printf '\xee\xaf\xa4' ;;                       # U+EBE4
	esac
}

# ---------------------------------------------------------------- primitives

# rx-bytes rx-errs rx-drop tx-bytes tx-errs tx-drop
counters() {
	awk -v d="$1" '{
		sub(/^ +/, ""); split($0, a, ":"); gsub(/ /, "", a[1])
		if (a[1] == d) { split(a[2], b, " "); print b[1], b[3], b[4], b[9], b[11], b[12] }
	}' /proc/net/dev
}

# Rates come from the elapsed time recorded in the state file, so they stay
# correct no matter what `interval` waybar is using.
throughput() { # iface -> "↓ 1.2 MB/s  ↑ 340 kB/s"
	_f="${XDG_RUNTIME_DIR:-/tmp}/waybar-net-$1"
	_now=$(awk '{print $1}' /proc/uptime)
	# shellcheck disable=SC2046  # deliberate word-split of counter fields
	set -- $(counters "$1")
	[ $# -ge 4 ] || { printf '↓ —  ↑ —'; return; }
	_rx=$1 _tx=$4 _drx=0 _dtx=0
	if [ -r "$_f" ]; then
		_pt=""
		read -r _pt _prx _ptx <"$_f" 2>/dev/null || _pt=""
		if [ -n "$_pt" ]; then
			# shellcheck disable=SC2046  # deliberate word-split of awk output
			set -- $(awk -v t="$_now" -v p="$_pt" -v r="$_rx" -v pr="$_prx" -v x="$_tx" -v px="$_ptx" 'BEGIN {
				d = t - p
				# counters reset on reboot or an iface flap -> report 0, not a spike
				if (d < 0.2 || r < pr || x < px) print 0, 0
				else print int((r - pr) / d), int((x - px) / d)
			}')
			_drx=$1 _dtx=$2
		fi
	fi
	printf '%s %s %s\n' "$_now" "$_rx" "$_tx" >"$_f"
	printf '↓ %s  ↑ %s' "$(human "$_drx")" "$(human "$_dtx")"
}

# ------------------------------------------------------------- network facts

# tab-separated: type<TAB>uuid<TAB>device<TAB>name (name last so a literal ":"
# in it survives nmcli's escaping)
active_tunnels() {
	nmcli -t -f TYPE,UUID,DEVICE,NAME connection show --active 2>/dev/null |
		while IFS=: read -r type uuid device name; do
			case "$type" in
			wireguard | vpn | tun) printf '%s\t%s\t%s\t%s\n' "$type" "$uuid" "$device" "$name" ;;
			esac
		done
}

# uuid<TAB>type<TAB>name for every saved tunnel the vendor apps didn't generate
vpn_profiles() {
	nmcli -t -f TYPE,UUID,NAME connection show 2>/dev/null |
		while IFS=: read -r type uuid name; do
			case "$type" in
			wireguard | vpn) printf '%s\t%s\t%s\n' "$uuid" "$type" "$name" ;;
			esac
		done | grep -Ev "$TAB$TAB?$EXCLUDE_RE" || true
}

# dev<TAB>kind<TAB>nm-profile-name for every tunnel interface that is actually up.
#
# Derived from the kernel, NOT from nmcli: for an OpenVPN connection nmcli
# reports DEVICE (and GENERAL.IP-IFACE) as the link the tunnel rides on —
# "wlo1" — never the tun0 it creates. Trusting that put the physical interface
# into the tunnel set, so every route through it graded as tunnelled and a
# wide-open box reported "encrypted end to end". The link kind is authoritative,
# and it also catches tunnels NM doesn't manage (wg-quick, a systemd openvpn
# unit). tap is excluded on purpose: those are VM bridges, not tunnels.
#
# The NM profile name is matched by IP rather than by device, since the address
# nmcli reports for the connection is the one that lands on the real interface.
tunnel_rows() {
	_map=$(nmcli -t -f TYPE,UUID,NAME connection show --active 2>/dev/null |
		while IFS=: read -r ty uu nm; do
			case "$ty" in
			wireguard | vpn | tun)
				_a=$(nmcli -g IP4.ADDRESS connection show uuid "$uu" 2>/dev/null | head -1 | cut -d/ -f1)
				[ -n "$_a" ] && printf '%s\t%s\n' "$_a" "$nm"
				;;
			esac
		done)
	ip -d -j link show 2>/dev/null | jq -r '
		.[] | select(.linkinfo.info_kind // "" | test("^(wireguard|tun|ppp|vti6?|xfrm)$"))
		    | select(.flags | index("UP")) | "\(.ifname)\t\(.linkinfo.info_kind)"' 2>/dev/null |
		while IFS="$TAB" read -r dev kind; do
			_a=$(ip -j -4 addr show dev "$dev" 2>/dev/null | jq -r '.[0].addr_info[0].local // empty' 2>/dev/null)
			_nm=$(printf '%s\n' "$_map" | awk -F'\t' -v a="${_a:-}" 'a != "" && $1 == a {print $2; exit}')
			printf '%s\t%s\t%s\n' "$dev" "$kind" "$_nm"
		done
}

# iface carrying general traffic for a family, "" when the family has no route
route_dev() { ip -j route get "$1" 2>/dev/null | jq -r '.[0].dev // empty' 2>/dev/null; }
gw_for() { ip -j "$2" route show default 2>/dev/null | jq -r --arg d "$1" '[.[] | select(.dev == $d)][0].gateway // empty' 2>/dev/null; }

# Pairwise prefix-overlap scan of the routing table.
#
# Two routes conflict when their prefixes overlap but point at different
# interfaces: the more specific one (or, when they are identical, the lower
# metric) silently wins, so traffic you believe is going down one tunnel goes
# down another. Overlaps on the *same* interface are ordinary subnetting and
# are ignored.
#
# Default routes are the exception the whole thing has to allow for: several
# are normal — that is how a VPN takes over — as long as their metrics differ.
# Only equal metrics are reported, because then the winner is arbitrary.
#
# Prefixes are compared as binary strings, which is exact for both families at
# any prefix length, and needs no 128-bit arithmetic.
# ponytail: O(n²) over the routing table — a couple of hundred comparisons on a
# normal host. If a full BGP table ever lands here, sort by prefix and compare
# neighbours instead.
conflict_scan() {
	awk -F'\t' '
	BEGIN { split("0000 0001 0010 0011 0100 0101 0110 0111 1000 1001 1010 1011 1100 1101 1110 1111", nib, " ") }
	function bits8(v,   i, s) { s = ""; for (i = 7; i >= 0; i--) s = s (int(v / 2^i) % 2); return s }
	function b4(a,   p, i, s) { split(a, p, "."); s = ""; for (i = 1; i <= 4; i++) s = s bits8(p[i] + 0); return s }
	function hex16(h,   i, s, v) {
		while (length(h) < 4) h = "0" h
		s = ""
		for (i = 1; i <= 4; i++) { v = index("0123456789abcdef", tolower(substr(h, i, 1))); s = s nib[v] }
		return s
	}
	function b6(a,   at, l, r, ln, rn, i, s, L, R) {
		at = index(a, "::")
		if (at) { l = substr(a, 1, at - 1); r = substr(a, at + 2) } else { l = a; r = "" }
		ln = (l == "" ? 0 : split(l, L, ":"))
		rn = (r == "" ? 0 : split(r, R, ":"))
		s = ""
		for (i = 1; i <= ln; i++) s = s hex16(L[i])
		for (i = 0; i < 8 - ln - rn; i++) s = s "0000000000000000"
		for (i = 1; i <= rn; i++) s = s hex16(R[i])
		return s
	}
	{
		fam = $1; dst = $2; dev = $3; met = $4 + 0
		if (dst == "default") { dn++; dfam[dn] = fam; ddev[dn] = dev; dmet[dn] = met; next }
		if (fam == 6 && (dst ~ /^fe80/ || dst ~ /^ff/)) next
		p = index(dst, "/")
		if (p) { addr = substr(dst, 1, p - 1); len = substr(dst, p + 1) + 0 }
		else   { addr = dst; len = (fam == 4 ? 32 : 128) }
		n++
		F[n] = fam; LEN[n] = len; DEV[n] = dev; MET[n] = met; DST[n] = dst
		BIN[n] = (fam == 4 ? b4(addr) : b6(addr))
	}
	END {
		for (i = 1; i <= n; i++) for (j = i + 1; j <= n; j++) {
			if (F[i] != F[j] || DEV[i] == DEV[j]) continue
			m = (LEN[i] < LEN[j] ? LEN[i] : LEN[j])
			if (substr(BIN[i], 1, m) != substr(BIN[j], 1, m)) continue
			kind = (LEN[i] == LEN[j] ? "identical" : "shadow")
			# the more specific route wins, so print it first
			a = (LEN[i] >= LEN[j] ? i : j); b = (LEN[i] >= LEN[j] ? j : i)
			printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\n", DST[a], DEV[a], MET[a], DST[b], DEV[b], MET[b], kind
		}
		for (i = 1; i <= dn; i++) for (j = i + 1; j <= dn; j++)
			if (dfam[i] == dfam[j] && dmet[i] == dmet[j])
				printf "default\t%s\t%s\tdefault\t%s\t%s\ttie\n", ddev[i], dmet[i], ddev[j], dmet[j]
	}'
}

route_conflicts() {
	{
		ip -j -4 route show 2>/dev/null | jq -r '.[] | select(.dev != "lo") | "4\t\(.dst)\t\(.dev)\t\(.metric // 0)"' 2>/dev/null
		ip -j -6 route show 2>/dev/null | jq -r '.[] | select(.dev != "lo") | "6\t\(.dst)\t\(.dev)\t\(.metric // 0)"' 2>/dev/null
	} | conflict_scan
}

nameservers() { awk '/^nameserver/ {print $2}' /etc/resolv.conf 2>/dev/null; }
searchdomains() { awk '/^search/ {$1 = ""; print substr($0, 2)}' /etc/resolv.conf 2>/dev/null; }

# Which interface a resolver is reached through. A loopback address is some
# local stub (dnscrypt-proxy, stubby, a container resolver) whose upstream is
# invisible from here — reported as "local" and counted as satisfied.
resolver_dev() {
	case "$1" in 127.* | ::1) printf 'local'; return ;; esac
	ip -j route get "$1" 2>/dev/null | jq -r '.[0].dev // "-"' 2>/dev/null
}

wifi_row() { nmcli -t -f ACTIVE,SECURITY,SSID dev wifi 2>/dev/null | awk -F: '$1 == "yes" {print; exit}'; }
wifi_ssid() { printf '%s' "$1" | sed 's/^yes:[^:]*://; s/\\:/:/g'; }
wifi_seclabel() { printf '%s' "$1" | cut -d: -f2; }

# open | wep | wpa | wired | none — the encryption of the physical uplink, not
# of whatever tunnel rides on top of it.
link_sec() {
	case "$(wifi_seclabel "$1")" in
	'' | '--') [ -n "$1" ] && { printf 'open'; return; } ;;
	*WEP*) printf 'wep'; return ;;
	*) printf 'wpa'; return ;;
	esac
	[ "$(cat "/sys/class/net/$ETH_DEV/carrier" 2>/dev/null)" = 1 ] && { printf 'wired'; return; }
	printf 'none'
}

# ------------------------------------------------------- the state machine
#
# Pure function: no commands, no globals, everything arrives as arguments so
# --selftest can drive it. First match wins.
#
#   conn     full | limited | portal | none | unknown
#   sec      open | wep | wpa | wired | none
#   v4/v6    iface carrying that family's default route, "" if the family has
#            no route at all (which is not a leak — it contributes nothing)
#   tuns     space-separated active tunnel interfaces
#   dnsdevs  space-separated interfaces the resolvers route out of, "local"
#            for a loopback stub
#   nconf    number of routing-table conflicts (see conflict_scan). Ranked
#            below the leaks: a conflict misroutes traffic between tunnels,
#            which matters, but not as much as traffic leaving in the clear.
#            It is always listed in the tooltip regardless of the class.
classify() {
	_conn=$1 _sec=$2 _v4=$3 _v6=$4 _tuns=$5 _dns=$6 _nconf=${7:-0}

	_is_tun() {
		for _t in $_tuns; do [ "$1" = "$_t" ] && return 0; done
		return 1
	}

	[ -z "$_v4" ] && [ -z "$_v6" ] && { printf 'offline'; return; }
	case "$_conn" in
	none) printf 'offline'; return ;;
	portal | limited) printf 'portal'; return ;;
	esac

	# Every family that HAS a default route must ride a tunnel. A v4-tunnelled
	# / v6-direct box is the classic IPv6 leak and lands here too.
	for _d in $_v4 $_v6; do
		if ! _is_tun "$_d"; then
			case "$_sec" in
			open | wep) printf 'open' ;;
			*) printf 'exposed' ;;
			esac
			return
		fi
	done

	for _d in $_dns; do
		[ "$_d" = local ] && continue
		_is_tun "$_d" || { printf 'dnsleak'; return; }
	done

	[ "$_nconf" -gt 0 ] && { printf 'conflict'; return; }
	printf 'secure'
}

# --------------------------------------------------------------- --sec

is_tun_dev() {
	[ -n "$1" ] || return 1
	for t in $TUNS; do [ "$1" = "$t" ] && return 0; done
	return 1
}

sec_emit() {
	CONN=$(nmcli -t -f CONNECTIVITY general 2>/dev/null | head -1)
	WROW=$(wifi_row)
	SEC=$(link_sec "$WROW")
	V4=$(route_dev "$PROBE4")
	V6=$(route_dev "$PROBE6")
	TROWS=$(tunnel_rows)
	TUNS=$(printf '%s\n' "$TROWS" | cut -f1 | grep -v '^$' | tr '\n' ' ')
	case "$SEC" in
	open | wep | wpa) UPLINK=$WIFI_DEV ;;
	wired) UPLINK=$ETH_DEV ;;
	*) UPLINK=${V4:-$WIFI_DEV} ;;
	esac
	DNS=$(nameservers)
	DNSDEVS=""
	for ns in $DNS; do DNSDEVS="$DNSDEVS $(resolver_dev "$ns")"; done
	CONF=$(route_conflicts)
	NCONF=$(printf '%s' "$CONF" | grep -c . || true)
	CLASS=$(classify "$CONN" "$SEC" "$V4" "$V6" "$TUNS" "$DNSDEVS" "$NCONF")

	case "$CLASS" in
	offline) HEAD="Offline — no route to the internet"; ICO=$(ic_offline) ;;
	portal) HEAD="Captive portal — click to sign in"; ICO=$(ic_portal) ;;
	open) HEAD="Unencrypted link — traffic in the clear"; ICO=$(ic_open) ;;
	exposed) HEAD="No tunnel — traffic leaves in the clear"; ICO=$(ic_exposed) ;;
	dnsleak) HEAD="Tunnelled, but DNS leaks"; ICO=$(ic_lock) ;;
	conflict) HEAD="Encrypted, but routes overlap"; ICO=$(ic_conflict) ;;
	secure) HEAD="Encrypted end to end"; ICO=$(ic_lock) ;;
	esac

	# CLASS/ICO above are recomputed every call — that lock icon has to stay
	# live. The tooltip body (route conflicts, tunnel rows, resolver rows…) is
	# cached on the state that actually drives it, not a timer: routes,
	# tunnels and resolvers only change on a real network event, which
	# net-watch.sh already turns into an immediate re-exec (RTMIN+10). A
	# time-based TTL would either serve a stale tooltip after a real change
	# within the same window, or rebuild for no reason when nothing did — this
	# has zero staleness and a near-total cache hit rate on an idle network.
	TIP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-net-sec-tip"
	TIP_KEY=$(printf '%s' "$CLASS$CONN$SEC$V4$V6$TROWS$CONF$DNS$DNSDEVS" | tr '\n\t' '  ')
	if tip_stale "$TIP_CACHE" "$TIP_KEY"; then
	TIP=$(
		title "$HEAD"
		rule 56

		sect "󰌾" "Link"
		case "$SEC" in
		open | wep | wpa)
			row "$(wifi_ssid "$WROW" | esc) · $WIFI_DEV"
			case "$SEC" in
			open) row "$(bad "Open — no encryption, anyone nearby can read this")" ;;
			wep) row "$(bad "WEP — broken, treat it as open")" ;;
			*) dim "$(wifi_seclabel "$WROW")" ;;
			esac
			MFP=$(iw dev "$WIFI_DEV" station dump 2>/dev/null | awk '/MFP:/ {print $2; exit}')
			dim "Management-frame protection (802.11w): ${MFP:-unknown}"
			;;
		wired)
			row "Ethernet · $ETH_DEV"
			dim "Wired — the link itself is not encrypted"
			;;
		*) dim "no uplink" ;;
		esac

		sect "󰖟" "Routes"
		if [ -n "$V4" ]; then
			GW=$(gw_for "$V4" -4)
			if is_tun_dev "$V4"; then
				row "IPv4  $(good ✓)  via $V4${GW:+ → $GW}"
			else
				row "IPv4  $(bad ✗)  via $V4${GW:+ → $GW} — not tunnelled"
			fi
		else
			dim "IPv4  no default route"
		fi
		if [ -n "$V6" ]; then
			GW6=$(gw_for "$V6" -6)
			if is_tun_dev "$V6"; then
				row "IPv6  $(good ✓)  via $V6${GW6:+ → $GW6}"
			else
				row "IPv6  $(bad ✗)  via $V6${GW6:+ → $GW6} — leaking outside the tunnel"
			fi
		else
			# scoped to the physical uplink: a ULA on the tunnel itself is not
			# evidence that the local network handed us usable IPv6
			N6=$(ip -j -6 addr show dev "$UPLINK" scope global 2>/dev/null | jq -r '[.[].addr_info[]?] | length' 2>/dev/null)
			if [ "${N6:-0}" -gt 0 ]; then
				dim "IPv6  address on $UPLINK but no default route — unused, not leaking"
			else
				dim "IPv6  none"
			fi
		fi

		# Only rendered when something actually overlaps, so the normal case
		# stays short. Listed whatever the class is — a conflict under a red
		# lock still needs to be visible.
		if [ -n "$CONF" ]; then
			sect "󰘬" "Route conflicts"
			printf '%s\n' "$CONF" | while IFS="$TAB" read -r da va ma db vb mb kind; do
				case "$kind" in
				tie)
					row "<tt>$(bad ✗)</tt>  two default routes both at metric $ma"
					dim "$va and $vb — which one wins is arbitrary, set distinct metrics"
					;;
				identical)
					row "<tt>$(bad ✗)</tt>  $da is announced by both $va and $vb"
					dim "metric $ma beats $mb, so $vb never sees this traffic"
					;;
				*)
					row "<tt>$(warn !)</tt>  $da via $va sits inside $db via $vb"
					dim "the more specific route wins, so that range leaves via $va, not $vb"
					;;
				esac
			done
		fi

		sect "󰦝" "Tunnels"
		if [ -z "$TROWS" ]; then
			dim "none active"
		else
			printf '%s\n' "$TROWS" | while IFS="$TAB" read -r dv kind nm; do
				[ -n "$dv" ] || continue
				case "$kind" in wireguard) TY=WireGuard ;; tun) TY=OpenVPN ;; *) TY=$kind ;; esac
				if is_tun_dev "$V4" && [ "$dv" = "$V4" ]; then
					CARRIES=" $(good "— carries the default route")"
				else
					CARRIES=""
				fi
				row "$(printf '%s · %s · %s%s' "$(printf '%s' "${nm:-unmanaged}" | esc)" "$dv" "$TY" "$CARRIES")"
			done
			CARRY=0
			for d in $V4 $V6; do is_tun_dev "$d" && CARRY=1; done
			[ "$CARRY" = 1 ] || row "$(warn "split tunnel — carries no default route")"
		fi

		sect "󰇖" "Resolvers"
		if [ -z "$DNS" ]; then
			dim "none configured"
		else
			for ns in $DNS; do
				d=$(resolver_dev "$ns")
				# <tt> for the address column: Google Sans Flex pads
				# proportionally, so %-20s alone does not line anything up.
				# 20 cells covers all but the longest uncompressed IPv6
				# literals; the old 32 pushed the "plaintext to the local
				# network" row past the tooltip's wrap width.
				NSP="<tt>$(printf '%-20s' "$ns")</tt>"
				if [ "$d" = local ]; then
					row "$(warn ~)  $NSP local stub"
				elif is_tun_dev "$d"; then
					row "$(good ✓)  $NSP $d"
				else
					row "$(bad ✗)  $NSP $d — plaintext to the local network"
				fi
			done
			SD=$(searchdomains | esc)
			[ -n "$SD" ] && dim "search $SD"
		fi

		if [ "$CLASS" = portal ]; then
			sect "󰋼" "Portal"
			dim "connectivity: $CONN — click to open the login page"
		fi
	)
	tip_save "$TIP_CACHE" "$TIP_KEY" "$TIP"
	else
		TIP=$(tip_load "$TIP_CACHE")
	fi
	# eco is a second, independent class rather than a different CLASS value —
	# TIP_KEY and the offline/portal/... case above must stay keyed on the
	# real state, not on which power mode drew it. style.css uses the pair
	# (.eco.open, .eco.portal) to hold this lock statically red instead of
	# pulsing: an infinite repaint loop is exactly what eco exists to cut, and
	# this is the one pill where "stay red" matters more than "look alive".
	EMIT_CLASS=$CLASS
	[ "$(power_mode)" = eco ] && EMIT_CLASS="$CLASS eco"
	emit "$EMIT_CLASS" "$(barico "$ICO")" "$TIP"
}

sec_click() {
	case "$(nmcli -t -f CONNECTIVITY general 2>/dev/null | head -1)" in
	portal | limited)
		# NM does not expose the portal's URL; a plaintext request is exactly
		# what every captive portal is built to hijack.
		xdg-open http://neverssl.com >/dev/null 2>&1 &
		return
		;;
	esac

	ACT=$(active_tunnels)
	PROF=$(vpn_profiles)
	[ -n "$PROF" ] || { notify "No VPN profiles saved"; return; }

	# Labels are built in awk for the same reason wifi-menu.sh does it: tab is
	# an IFS whitespace character, so `read` collapses empty fields.
	MENU=$(printf '%s\n@@@\n%s\n' "$ACT" "$PROF" | awk -F'\t' '
		function esc(s) { gsub(/&/, "\\&amp;", s); gsub(/</, "\\&lt;", s); gsub(/>/, "\\&gt;", s); return s }
		$0 == "@@@" { prof = 1; next }
		!prof { if ($2 != "") dev[$2] = ($3 == "" ? "up" : $3); next }
		$1 != "" {
			ty = ($2 == "wireguard" ? "WireGuard" : "OpenVPN")
			mark = ($1 in dev) ? "  (connected · " dev[$1] ")" : ""
			printf "<tt><span alpha=\"65%%\">%-10s</span></tt> %s<span alpha=\"55%%\">%s</span>\n", ty, esc($3), mark
		}')

	RULE='<span alpha="30%">────────────────────</span>'
	IDX=$(printf '%s\n%s\n  Connection settings…\n' "$MENU" "$RULE" |
		rofi -dmenu -format i -markup-rows -p "VPN" \
			-mesg "Enter toggles the tunnel" \
			-theme-str 'window { width: 720px; } listview { spacing: 5px; } element { padding: 9px 8px; }') || return
	case "$IDX" in '' | *[!0-9]*) return ;; esac

	N=$(printf '%s\n' "$PROF" | grep -c . || true)
	if [ "$IDX" -ge "$N" ]; then
		[ "$IDX" = "$((N + 1))" ] && nm-connection-editor &
		return
	fi

	ROW=$(printf '%s\n' "$PROF" | sed -n "$((IDX + 1))p")
	UUID=$(printf '%s' "$ROW" | cut -f1)
	NAME=$(printf '%s' "$ROW" | cut -f3)

	if printf '%s\n' "$ACT" | cut -f2 | grep -qxF "$UUID"; then
		OUT=$(nmcli connection down uuid "$UUID" 2>&1) || notify "Failed to disconnect $NAME: $OUT"
	else
		OUT=$(nmcli connection up uuid "$UUID" 2>&1) || notify "Failed to connect $NAME: $OUT"
	fi
	pkill -RTMIN+10 waybar 2>/dev/null || true
}

sec_edit() {
	UUID=$(active_tunnels | head -1 | cut -f2)
	if [ -n "$UUID" ]; then
		nm-connection-editor --edit="$UUID" &
	else
		nm-connection-editor &
	fi
}

# ------------------------------------------------ shared addressing block

addr_rows() {
	dev=$1
	V4A=$(ip -j -4 addr show dev "$dev" 2>/dev/null | jq -r '.[0].addr_info[]? | "\(.local)/\(.prefixlen)"' 2>/dev/null | head -1)
	GWA=$(gw_for "$dev" -4)
	V6A=$(ip -j -6 addr show dev "$dev" scope global 2>/dev/null | jq -r '.[0].addr_info[]? | "\(.local)/\(.prefixlen)"' 2>/dev/null | head -1)
	GW6A=$(gw_for "$dev" -6)
	MTU=$(cat "/sys/class/net/$dev/mtu" 2>/dev/null)
	MAC=$(cat "/sys/class/net/$dev/address" 2>/dev/null)

	if [ -n "$V4A" ]; then row "IPv4  $V4A${GWA:+  → $GWA}"; else dim "IPv4  none"; fi
	if [ -n "$V6A" ]; then row "IPv6  $V6A${GW6A:+  → $GW6A}"; else dim "IPv6  none"; fi
	dim "MAC ${MAC:-?} · MTU ${MTU:-?}"
	NS=$(nameservers | paste -sd' ' -)
	[ -n "$NS" ] && dim "DNS $NS"
	SD=$(searchdomains | esc)
	[ -n "$SD" ] && dim "search $SD"
	return 0
}

# --------------------------------------------------------------- --wifi

# RSSI in dBm -> 0..100. -90 dBm is unusable, -30 is standing next to the AP.
rssi_pct() { awk -v r="$1" 'BEGIN { p = (r + 90) * 100 / 60; print (p > 100 ? 100 : (p < 0 ? 0 : int(p))) }'; }
rssi_label() {
	awk -v r="$1" 'BEGIN {
		print (r >= -50 ? "Excellent" : (r >= -60 ? "Good" : (r >= -70 ? "Fair" : (r >= -80 ? "Weak" : "Very weak"))))
	}'
}

# NM device states that mean "mid-transition": 40 prepare, 50 config,
# 60 need-auth, 70 ip-config, 80 ip-check, 90 secondaries, 110 deactivating.
# The number is never localized; the parenthetical name is, so don't read it.
wifi_busy() { # "<code> (<name>)"
	case "${1%% *}" in
	40 | 50 | 60 | 70 | 80 | 90 | 110) return 0 ;;
	*) return 1 ;;
	esac
}

wifi_emit() {
	# Whenever custom/netwatch is drawing a spinner in this slot (net-watch.sh),
	# emit nothing — waybar hides a custom module with empty text — so the
	# spinner *replaces* the arc instead of appearing next to it. Two causes:
	# an NM state change, and wifi-menu.sh's rescan, which changes no device
	# state and so has to announce itself. Its flag holds its pid, so a scan
	# that died without cleaning up cannot hide the widget forever.
	SCAN=$(cat "$SCAN_FLAG" 2>/dev/null)
	if [ -n "$SCAN" ] && kill -0 "$SCAN" 2>/dev/null; then
		emit busy "" ""
		return
	fi
	# Also skips the ~190ms of `iw` work that has no station to read yet.
	if wifi_busy "$(nmcli -g GENERAL.STATE device show "$WIFI_DEV" 2>/dev/null)"; then
		emit busy "" ""
		return
	fi

	DUMP=$(iw dev "$WIFI_DEV" station dump 2>/dev/null)
	LINK=$(iw dev "$WIFI_DEV" link 2>/dev/null)

	if [ -z "$DUMP" ] || [ "${LINK#Not connected}" != "$LINK" ]; then
		if [ "$(nmcli radio wifi 2>/dev/null)" = disabled ]; then
			TIP=$(title "Wi-Fi off"; dim "radio disabled — click to enable")
		else
			TIP=$(title "Wi-Fi disconnected"; dim "$WIFI_DEV — click to pick a network")
		fi
		emit disconnected "$(barico "$(ic_wifioff)")" "$TIP"
		return
	fi

	fld() { printf '%s\n' "$DUMP" | awk -v k="$1" -v n="$2" '$0 ~ k { print $n; exit }'; }
	RSSI=$(fld '^\tsignal:' 2)
	AVG=$(fld 'signal avg:' 3)
	BCN=$(fld 'beacon signal avg:' 4)
	MFP=$(fld 'MFP:' 2)
	RETRY=$(fld 'tx retries:' 3)
	FAILED=$(fld 'tx failed:' 3)
	BLOSS=$(fld 'beacon loss:' 3)
	RXDROP=$(fld 'rx drop misc:' 4)
	UPTIME=$(fld 'connected time:' 3)
	RXRATE=$(printf '%s\n' "$DUMP" | awk '/rx bitrate:/ { sub(/^[^:]*:[ \t]*/, ""); print; exit }')
	TXRATE=$(printf '%s\n' "$DUMP" | awk '/tx bitrate:/ { sub(/^[^:]*:[ \t]*/, ""); print; exit }')
	BSSID=$(printf '%s\n' "$LINK" | awk '/^Connected to/ { print $3; exit }')
	FREQ=$(printf '%s\n' "$LINK" | awk '/^\tfreq:/ { print $2; exit }')
	SSID=$(printf '%s\n' "$LINK" | awk '/^\tSSID:/ { sub(/^[^:]*:[ \t]*/, ""); print; exit }')

	PCT=$(rssi_pct "${RSSI:--90}")
	if [ "$PCT" -ge 60 ]; then
		COL=$C_GOOD CLASS=excellent
	elif [ "$PCT" -ge 35 ]; then
		COL=$C_WARN CLASS=good
	else
		COL=$C_BAD CLASS=weak
	fi

	WROW=$(wifi_row)
	SECRAW=$(link_sec "$WROW")

	# CLASS/PCT/the arc above stay live every call; only the tooltip body is
	# cached — see the matching comment in sec_emit.
	TIP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-net-wifi-tip"
	TIP_KEY="$CLASS-$(tip_bucket 30)"
	if tip_stale "$TIP_CACHE" "$TIP_KEY"; then
	TIP=$(
		title "$(printf '%s' "${SSID:-Wi-Fi}" | esc)"
		rule 50

		sect "󰤨" "Signal"
		row "$(bar "$PCT" "$COL")  ${RSSI:-?} dBm"
		dim "$(rssi_label "${RSSI:--90}") · $PCT% · avg ${AVG:-?} dBm · beacon ${BCN:-?} dBm"

		sect "󰌾" "Security"
		case "$SECRAW" in
		open) row "$(bad "Open — unencrypted, anyone nearby can read your traffic")" ;;
		wep) row "$(bad "WEP — broken, treat it as open")" ;;
		*) row "$(good "$(wifi_seclabel "$WROW")")" ;;
		esac
		case "$MFP" in
		yes) dim "Management-frame protection (802.11w): $(good yes)" ;;
		*) dim "Management-frame protection (802.11w): $(warn "${MFP:-no}") — deauth attacks possible" ;;
		esac

		sect "󰖩" "Radio"
		BAND=$(awk -v m="${FREQ:-0}" 'BEGIN { print (m >= 5925 ? "6 GHz" : (m >= 4900 ? "5 GHz" : "2.4 GHz")) }')
		CH=$(awk -v m="${FREQ:-0}" 'BEGIN {
			if (m >= 5925) c = int((m - 5950) / 5)
			else if (m >= 4900) c = int((m - 5000) / 5)
			else c = int((m - 2407) / 5)
			print c
		}')
		WIDTH=$(printf '%s' "$RXRATE" | grep -oE '[0-9]+MHz' | head -1)
		row "$BAND · channel $CH · ${FREQ:-?} MHz${WIDTH:+ · $WIDTH}"
		dim "BSSID ${BSSID:-?} · $WIFI_DEV"

		sect "󰓅" "Throughput"
		row "$(throughput "$WIFI_DEV")"
		dim "link ↓ ${RXRATE:-?}"
		dim "link ↑ ${TXRATE:-?}"

		sect "󰋼" "Link quality"
		dim "tx retries ${RETRY:-0} · tx failed ${FAILED:-0}"
		dim "beacon loss ${BLOSS:-0} · rx drop ${RXDROP:-0}"
		dim "connected $(awk -v s="${UPTIME:-0}" 'BEGIN {
			h = int(s / 3600); m = int(s % 3600 / 60)
			if (h) printf "%dh %dm", h, m; else printf "%dm", m
		}')"

		sect "󰩟" "Addressing"
		addr_rows "$WIFI_DEV"
	)
	tip_save "$TIP_CACHE" "$TIP_KEY" "$TIP"
	else
		TIP=$(tip_load "$TIP_CACHE")
	fi
	emit "$CLASS" "$(barico "$(arc "$PCT")") $PCT%" "$TIP"
}

# ---------------------------------------------------------------- --eth

eth_emit() {
	CARRIER=$(cat "/sys/class/net/$ETH_DEV/carrier" 2>/dev/null || echo 0)
	OPER=$(cat "/sys/class/net/$ETH_DEV/operstate" 2>/dev/null || echo down)
	SPEED=$(cat "/sys/class/net/$ETH_DEV/speed" 2>/dev/null || echo -1)
	DUPLEX=$(cat "/sys/class/net/$ETH_DEV/duplex" 2>/dev/null || echo unknown)
	IP4=$(ip -j -4 addr show dev "$ETH_DEV" 2>/dev/null | jq -r '.[0].addr_info[0].local // empty' 2>/dev/null)

	if [ "$CARRIER" != 1 ]; then
		CLASS=disconnected TEXT='' ICO=$(ic_ethoff)
	elif [ -z "$IP4" ]; then
		CLASS=linked TEXT=' no IP' ICO=$(ic_eth)
	else
		CLASS=connected TEXT=" $IP4" ICO=$(ic_eth)
	fi

	# Same reasoning as sec_emit: carrier/speed/IP only change on a real link
	# event, which net-watch.sh already turns into an immediate re-exec, so
	# keying on that state beats a timer. The rx/tx counters in "Counters"
	# below are left out of the key deliberately — those climb every poll on
	# an active link and would defeat caching entirely if included; they are
	# the one part of this tooltip that stays only as fresh as the last real
	# link change or the 60s backstop interval.
	TIP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-net-eth-tip"
	TIP_KEY="$CLASS$CARRIER$OPER$SPEED$DUPLEX$IP4"
	if tip_stale "$TIP_CACHE" "$TIP_KEY"; then
	TIP=$(
		title "Ethernet · $ETH_DEV"
		rule 34

		sect "󰈀" "Link"
		if [ "$CARRIER" != 1 ]; then
			row "$(bad "down — no carrier")"
			dim "operstate $OPER · click to enable the adapter"
		else
			if [ "${SPEED:--1}" -gt 0 ] 2>/dev/null; then
				row "$(good up) · $SPEED Mbit/s · $DUPLEX duplex"
			else
				row "$(good up) · speed unknown"
			fi
			dim "operstate $OPER"
			dim "Wired — the link itself is not encrypted"
		fi

		sect "󰩟" "Addressing"
		addr_rows "$ETH_DEV"

		sect "󰓅" "Throughput"
		row "$(throughput "$ETH_DEV")"

		sect "󰋼" "Counters"
		# shellcheck disable=SC2046  # deliberate word-split of counter fields
		set -- $(counters "$ETH_DEV")
		if [ $# -ge 6 ]; then
			dim "rx errors $2 · rx drops $3"
			dim "tx errors $5 · tx drops $6"
		else
			dim "unavailable"
		fi
	)
	tip_save "$TIP_CACHE" "$TIP_KEY" "$TIP"
	else
		TIP=$(tip_load "$TIP_CACHE")
	fi
	emit "$CLASS" "$(barico "$ICO")$TEXT" "$TIP"
}

# ------------------------------------------------------------ --selftest

selftest() {
	fails=0
	check() { # expected, then classify's args (nconf defaults to 0)
		want=$1
		shift
		got=$(classify "$@")
		if [ "$got" = "$want" ]; then
			printf 'ok    %-8s  %s\n' "$got" "$*"
		else
			printf 'FAIL  want %-8s got %-8s  %s\n' "$want" "$got" "$*"
			fails=$((fails + 1))
		fi
	}

	#     expect    conn     sec    v4    v6     tunnels  dns-devs
	check offline "full" "wpa" "" "" "" ""
	check offline "none" "wpa" "wlo1" "" "" "wlo1"
	check portal "portal" "wpa" "wlo1" "" "" "wlo1"
	check portal "limited" "open" "wlo1" "" "" "wlo1"
	check open "full" "open" "wlo1" "" "" "wlo1"
	check open "full" "wep" "wlo1" "" "" "wlo1"
	# open AP with a tunnel that isn't carrying the default route (today's box)
	check open "full" "open" "wlo1" "" "wg0" "wg0 wlo1"
	check exposed "full" "wpa" "wlo1" "" "" "wlo1"
	check exposed "full" "wired" "eno2" "" "" "eno2"
	# split tunnel: tunnel up, default still on the physical link
	check exposed "full" "wpa" "wlo1" "" "wg0" "wg0"
	# the IPv6 leak: v4 tunnelled, v6 default still on the AP
	check exposed "full" "wpa" "wg0" "wlo1" "wg0" "wg0"
	# no v6 default route at all is not a leak
	check secure "full" "wpa" "wg0" "" "wg0" "wg0"
	check secure "full" "open" "wg0" "wg0" "wg0" "wg0 wg0"
	check secure "full" "wpa" "wg0" "" "wg0" "local"
	check secure "full" "wpa" "wg0" "wg0" "wg0 wg1" "wg1 wg0"
	check dnsleak "full" "wpa" "wg0" "" "wg0" "wg0 wlo1"
	check dnsleak "full" "wpa" "wg0" "wg0" "wg0" "wlo1"
	# route conflicts rank below the leaks, and never mask one
	check conflict "full" "wpa" "wg0" "" "wg0" "wg0" 2
	check dnsleak "full" "wpa" "wg0" "" "wg0" "wlo1" 2
	check open "full" "open" "wlo1" "" "" "wlo1" 3
	check secure "full" "wpa" "wg0" "" "wg0" "wg0" 0

	printf '\n-- wifi busy states --\n'
	busycheck() { # expected (busy|idle), GENERAL.STATE string
		if wifi_busy "$2"; then got=busy; else got=idle; fi
		if [ "$got" = "$1" ]; then
			printf 'ok    %-8s  %s\n' "$got" "$2"
		else
			printf 'FAIL  want %-8s got %-8s  %s\n' "$1" "$got" "$2"
			fails=$((fails + 1))
		fi
	}
	busycheck idle '100 (connected)'
	busycheck idle '30 (disconnected)'
	busycheck idle '20 (unavailable)'
	busycheck idle '10 (unmanaged)'
	busycheck idle '120 (failed)'
	busycheck busy '40 (connecting (prepare))'
	busycheck busy '70 (connecting (getting IP configuration))'
	busycheck busy '90 (connecting (starting secondary connections))'
	busycheck busy '110 (deactivating)'

	printf '\n-- route overlap scanner --\n'
	scan() { # expected line count, expected kind (or "-"), tab-separated routes
		want=$1 kind=$2 input=$3
		out=$(printf '%s\n' "$input" | conflict_scan)
		got=$(printf '%s' "$out" | grep -c . || true)
		gotkind=$(printf '%s' "$out" | head -1 | cut -f7)
		if [ "$got" = "$want" ] && { [ "$kind" = "-" ] || [ "$gotkind" = "$kind" ]; }; then
			printf 'ok    %d %-10s %s\n' "$got" "${gotkind:--}" "$(printf '%s' "$input" | tr '\n' ';')"
		else
			printf 'FAIL  want %s/%s got %s/%s  %s\n' "$want" "$kind" "$got" "${gotkind:--}" \
				"$(printf '%s' "$input" | tr '\n' ';')"
			fails=$((fails + 1))
		fi
	}

	R4() { printf '4\t%s\t%s\t%s' "$1" "$2" "${3:-0}"; }
	R6() { printf '6\t%s\t%s\t%s' "$1" "$2" "${3:-0}"; }

	# the case that prompted this: one VPN's subnet swallowed by another's
	scan 1 shadow "$(R4 10.10.0.0/20 tun0 1000)
$(R4 10.10.0.0/16 wg0 50)"
	scan 1 identical "$(R4 10.10.0.0/20 tun0 1000)
$(R4 10.10.0.0/20 wg0 50)"
	# ordinary subnetting on one interface is not a conflict
	scan 0 - "$(R4 10.10.0.0/20 tun0 1000)
$(R4 10.10.0.0/16 tun0 1000)"
	scan 0 - "$(R4 10.10.0.0/17 tun0)
$(R4 10.11.0.0/17 wg0)"
	# a host route landing inside someone else's subnet
	scan 1 shadow "$(R4 10.10.0.5 wg0 50)
$(R4 10.10.0.0/24 tun0 1000)"
	# several defaults are how a VPN takes over — only a metric tie is ambiguous
	scan 0 - "$(R4 default wlo1 600)
$(R4 default wg0 50)"
	scan 1 tie "$(R4 default wlo1 600)
$(R4 default wg0 600)"
	# IPv6, including the /64-inside-/32 case and link-local being ignored
	scan 1 shadow "$(R6 fd10:10:1::/64 wg0 50)
$(R6 fd10:10::/32 tun0 1000)"
	scan 0 - "$(R6 fd10:10:1::/64 wg0)
$(R6 fd10:10:2::/64 tun0)"
	scan 0 - "$(R6 fe80::/64 wg0)
$(R6 fe80::/64 tun0)"
	# families must never cross-match despite identical leading bits
	scan 0 - "$(R4 10.10.0.0/16 tun0)
$(R6 ::/0 wg0)"

	# Regression guard for the bug that graded a wide-open box "encrypted end to
	# end": nmcli reports an OpenVPN connection's device as the physical link it
	# rides on, so any tunnel set rebuilt from nmcli contains wlo1/eno2 — and
	# then every route through the physical interface reads as tunnelled.
	# classify() was never wrong here; the input was. This catches that.
	set=$(tunnel_rows | cut -f1 | tr '\n' ' ')
	for phys in "$WIFI_DEV" "$ETH_DEV"; do
		if printf ' %s ' "$set" | grep -q " $phys "; then
			printf 'FAIL  physical interface %s is in the tunnel set: %s\n' "$phys" "$set"
			fails=$((fails + 1))
		else
			printf 'ok    %-8s  not a tunnel (set: %s)\n' "$phys" "${set:-none}"
		fi
	done

	if [ "$fails" = 0 ]; then
		printf '\nall passed\n'
	else
		printf '\n%d failed\n' "$fails"
		return 1
	fi
}

case "${1:-}" in
--sec-click) sec_click ;;
--sec-edit) sec_edit ;;
--wifi) wifi_emit ;;
--eth) eth_emit ;;
--selftest) selftest ;;
*) sec_emit ;;
esac
