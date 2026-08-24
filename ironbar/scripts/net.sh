#!/bin/sh
# Network security lock click handler + selftest for ironbar.
#
# Three modes:
#   --sec-click  on-click (left)     — captive portal login, else the VPN picker
#   --sec-edit   on-click-right      — nm-connection-editor on the active tunnel
#   --selftest   assert classify() against canned inputs (no network access)
#
# The `netsec`/`wifi`/`eth` exec pollers that used to live here (grading the
# path, RSSI/link detail) moved to mango-bard's net.rs at the T8 cutover —
# see IRONBAR.md. genconfig.rs's netsec pill has no `exec` key, only the two
# click handlers below.
set -u

WIFI_DEV="${MANGO_WIFI_DEV:-wlo1}"
ETH_DEV="${MANGO_ETH_DEV:-eno2}"

# Vendor apps regenerate their profiles behind your back; the picker only
# offers tunnels you imported yourself. Matched against the profile name.
EXCLUDE_RE='(ProtonVPN|Nord|NordLynx|PVPN)'

TAB=$(printf '\t')

notify() { notify-send -a mango-bard "Network" "$1"; }

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

# NM device states that mean "mid-transition": 40 prepare, 50 config,
# 60 need-auth, 70 ip-config, 80 ip-check, 90 secondaries, 110 deactivating.
# The number is never localized; the parenthetical name is, so don't read it.
wifi_busy() { # "<code> (<name>)"
	case "${1%% *}" in
	40 | 50 | 60 | 70 | 80 | 90 | 110) return 0 ;;
	*) return 1 ;;
	esac
}

# --------------------------------------------------------------- --sec-click

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
	# No poke needed: `nmcli connection up/down` above is itself a real NM
	# event — net.rs's `nmcli monitor` child already regrades from it
	# (IRONBAR.md T3), the same as every other join/disconnect action.
}

sec_edit() {
	UUID=$(active_tunnels | head -1 | cut -f2)
	if [ -n "$UUID" ]; then
		nm-connection-editor --edit="$UUID" &
	else
		nm-connection-editor &
	fi
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
--selftest) selftest ;;
esac
