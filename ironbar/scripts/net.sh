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

# Written by system/vpnguard/mango-vpnguard, world-readable — no root
# needed to read it (net.rs's guard_state_to_verdict reads the same file).
GUARD_STATE_FILE="${MANGO_VG_STATE_FILE:-/run/mango-vpnguard/state}"
VG_BIN="${MANGO_VG_BIN:-mango-vpnguard}"

TAB=$(printf '\t')
RULE='<span alpha="30%">──────────────────────────────────────</span>'

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

# Action rows only, as action<TAB>arg<TAB>label — no status prose. The
# ironbar pill already says protected/not; the picker's job is what to do
# about it, not to repeat that. `action` is empty for a row Enter does
# nothing on; none of these carry an arg, so field 2 is always empty.
guard_action_rows() { # <state>
	case "$1" in
	vpn:*)
		printf 'disarm\t\t▸  Turn protection off\n'
		;;
	blocked)
		printf 'protect\t\t▸  Try again\n'
		printf 'unsecure\t\t▸  Go without protection\n'
		;;
	portal)
		# Rarely reached: sec_click's own connectivity check (below) already
		# intercepts the live case before the menu is ever built. This is the
		# fallback for a state file left over from a moment ago.
		printf 'portal-open\t\t▸  Open the sign-in page\n'
		;;
	# unsecured, unconfigured, and unknown/garbage (no state file — guard not
	# installed, or not armed this boot) all land here. Must NOT offer
	# "disarm" — that claims protection is on, which for an unknown state it
	# almost certainly is not.
	*)
		printf 'protect\t\t▸  Turn protection on\n'
		;;
	esac
}

# uuid<TAB>type<TAB>name for every WireGuard/OpenVPN profile NM knows about,
# unfiltered — a named function, not a $VAR binary like VG_BIN, so the
# selftest overrides it by name instead of stubbing a binary.
all_profiles() {
	nmcli -t -f TYPE,UUID,NAME connection show 2>/dev/null |
		while IFS=: read -r type uuid name; do
			case "$type" in
			wireguard | vpn) printf '%s\t%s\t%s\n' "$uuid" "$type" "$name" ;;
			esac
		done
}

# uuid<TAB>type<TAB>name for every profile in <group-key> — same prefix rule
# profile_rows uses to collapse rows (the name up to its first " (", or the
# whole name if it has none). Kept in one place so a click and a render can
# never disagree about membership.
#
# Not user.data-tagged: this NM build (1.58.1) rejects the `user` setting
# outright ("invalid or not allowed setting 'user'") on both a dummy and a
# real WireGuard connection, so there is nowhere to store an explicit tag.
group_members() { # <group-key>
	all_profiles | awk -F'\t' -v g="$1" '
		function grp(nm,   i) { i = index(nm, " ("); return i ? substr(nm, 1, i - 1) : nm }
		grp($3) == g { print }
	'
}

group_wg_names() { group_members "$1" | awk -F'\t' '$2 == "wireguard" { print $3 }'; }

# Every WireGuard/OpenVPN profile exactly once, as one flat
# action<TAB>arg<TAB>label table: `mango-vpnguard list` (already ordered —
# chain entries by descending priority, then always-on companions, see
# mango-vpnguard's cmd_list) supplies the "auto"/"always" rows; anything
# `all_profiles` knows that `list` didn't claim is "manual" — not yet in the
# chain (WireGuard: action "guard", opts it in) or out of vpnguard's remit
# entirely (OpenVPN: action "toggle", plain nmcli up/down). Live state comes
# from `active_tunnels` (passed in — sec_click already has one call's worth,
# no need for a second), matched by name — the same field OpenVPN and
# WireGuard both carry there.
#
# Profiles sharing a name prefix up to " (" — every "ProtonVPN (…)" — fold
# into one "group" row, so a roadwarrior VPN with several servers reads as
# one line, not one per server. A group's rank is its best (lowest-numbered,
# i.e. highest-priority) member's rank — guaranteed to be the first member
# seen below, since `list` already hands chain entries over in priority
# order, ahead of always-on, ahead of manual. The second argument is a
# TAB-separated list of expanded group keys: an expanded group keeps its
# header row (chevron ▾, action still "group") and lists its members
# directly below it, indented, with their real actions.
profile_rows() { # <active_tunnels output> [expanded groups, TAB-separated]
	list=$("$VG_BIN" list 2>/dev/null)
	act=$1
	all=$(all_profiles)

	printf '%s\n@@@\n%s\n@@@\n%s\n' "$list" "$act" "$all" | awk -F'\t' -v expanded="${2:-}" '
		function esc(s) { gsub(/&/, "\\&amp;", s); gsub(/</, "\\&lt;", s); gsub(/>/, "\\&gt;", s); return s }
		function grp(nm,   i) { i = index(nm, " ("); return i ? substr(nm, 1, i - 1) : nm }
		function emitrow(nm, pad,   live, action, arg, meta) {
			live = (nm in dev) ? "●" : "○"
			if (role[nm] == "manual" && kind[nm] == "OpenVPN") { action = "toggle"; arg = uuid[nm] }
			else if (role[nm] == "manual") { action = "guard"; arg = nm }
			else { action = "up"; arg = nm }
			meta = "  " kind[nm] " · " ((rank[nm] != "") ? "auto #" rank[nm] : role[nm])
			if (nm in dev) meta = meta " · connected · " dev[nm]
			printf "%s\t%s\t%s<tt><span alpha=\"65%%\">%s</span></tt>  %s<span alpha=\"55%%\">%s</span>\n",
				action, arg, pad, live, esc(nm), meta
		}
		$0 == "@@@" { sec++; next }
		# section 0: vg list — prio, role, dev, name (wireguard only)
		sec == 0 {
			if ($4 == "") next
			known[$4] = 1
			if ($2 == "default") { n++; role[$4] = "auto " n; rank[$4] = n }
			else if ($2 == "always") { role[$4] = "always" }
			else next
			kind[$4] = "WireGuard"
			order[++k] = $4
			next
		}
		# section 1: active tunnels — type, uuid, device, name
		sec == 1 {
			if ($4 != "") dev[$4] = ($3 == "" ? "up" : $3)
			next
		}
		# section 2: all profiles — uuid, type, name
		sec == 2 {
			if ($3 == "") next
			uuid[$3] = $1
			if ($3 in known) next
			role[$3] = "manual"
			kind[$3] = ($2 == "wireguard") ? "WireGuard" : "OpenVPN"
			order[++k] = $3
			next
		}
		END {
			# Size each group first: its first-seen slot, member count, and
			# the rank/kind/live state of its best member.
			for (i = 1; i <= k; i++) {
				nm = order[i]; g = grp(nm)
				if (!(g in gpos)) gpos[g] = i
				gcount[g]++
				if (!(g in gkind)) gkind[g] = kind[nm]
				if (nm in dev) { glive[g] = 1; if (!(g in gdev)) gdev[g] = dev[nm] }
				if (rank[nm] != "" && !(g in grank)) grank[g] = rank[nm]
				else if (role[nm] == "always" && !(g in grank)) galways[g] = 1
			}

			nexp = split(expanded, E, "\t")
			for (i = 1; i <= nexp; i++) if (E[i] != "") isexp[E[i]] = 1

			for (i = 1; i <= k; i++) {
				nm = order[i]; g = grp(nm)
				if (gcount[g] > 1) {
					if (i != gpos[g]) continue # rendered at the header slot
					live = (g in glive) ? "●" : "○"
					role_txt = (g in grank) ? "auto #" grank[g] : (g in galways) ? "always" : "manual"
					meta = "  " gkind[g] " · " role_txt " · " gcount[g] " servers"
					if (g in glive) meta = meta " · connected · " gdev[g]
					chev = (g in isexp) ? "▾" : "▸"
					printf "group\t%s\t<tt><span alpha=\"65%%\">%s</span></tt>  %s %s<span alpha=\"55%%\">%s</span>\n",
						g, live, chev, esc(g), meta
					if (g in isexp)
						for (j = 1; j <= k; j++)
							if (grp(order[j]) == g) emitrow(order[j], "    ")
					continue
				}
				emitrow(nm, "")
			}
		}'
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

# <name> <priority-delta> — nmcli connection.autoconnect-priority, floored at
# 1 on the way down: Ctrl+Enter, not this, is how a chain entry goes manual.
reorder_one() {
	cur=$(nmcli -g connection.autoconnect-priority connection show "$1" 2>/dev/null)
	case "$cur" in '' | *[!0-9]*) cur=0 ;; esac
	new=$((cur + $2))
	if [ "$2" -lt 0 ] && [ "$new" -le 0 ]; then
		new=1
	fi
	nmcli connection modify "$1" connection.autoconnect-priority "$new" >/dev/null 2>&1
}

# <name> -> nmcli's own output (empty on success). A chain entry (priority>0)
# drops to 0; an always-on companion (priority 0, autoconnect=yes) is the one
# case priority alone can't clear, so autoconnect is what's turned off there.
manual_one() {
	cur=$(nmcli -g connection.autoconnect-priority connection show "$1" 2>/dev/null)
	case "$cur" in '' | *[!0-9]*) cur=0 ;; esac
	if [ "$cur" -gt 0 ]; then
		nmcli connection modify "$1" connection.autoconnect-priority 0 2>&1
	else
		nmcli connection modify "$1" connection.autoconnect no 2>&1
	fi
}

# Dials the highest-priority WireGuard member of a group that gets a real
# handshake — the group form of mango-vpnguard's own first-success chain
# walk. If none of the group is opted in yet, opts every member in at a
# fresh descending priority first: the group form of the single-row "guard"
# action.
group_connect() { # <group-key>
	g=$1
	wg=$(group_wg_names "$g")
	[ -n "$wg" ] || {
		notify "No WireGuard profile in $g"
		return
	}

	oldIFS=$IFS
	IFS='
'
	set -f
	pri=$(for nm in $wg; do
		[ -n "$nm" ] || continue
		p=$(nmcli -g connection.autoconnect-priority connection show "$nm" 2>/dev/null)
		case "$p" in '' | *[!0-9]*) p=0 ;; esac
		printf '%s\t%s\n' "$p" "$nm"
	done)

	if ! printf '%s\n' "$pri" | cut -f1 | grep -qv '^0$'; then
		p=$(($(printf '%s\n' "$wg" | grep -c .) * 10))
		for nm in $wg; do
			[ -n "$nm" ] || continue
			nmcli connection modify "$nm" connection.autoconnect-priority "$p" >/dev/null 2>&1
			p=$((p - 10))
		done
		pri=$(for nm in $wg; do
			[ -n "$nm" ] || continue
			p=$(nmcli -g connection.autoconnect-priority connection show "$nm" 2>/dev/null)
			printf '%s\t%s\n' "${p:-0}" "$nm"
		done)
	fi

	won=0
	for nm in $(printf '%s\n' "$pri" | sort -t "$TAB" -k1,1nr | cut -f2); do
		[ -n "$nm" ] || continue
		if OUT=$(printf 'CONN=%s\n' "$nm" | sudo mango-vpnguard up 2>&1); then
			notify "$(mango-vpnguard status)"
			won=1
			break
		fi
	done
	[ "$won" = 1 ] || notify "No server in $g came up"

	set +f
	IFS=$oldIFS
}

group_reorder() { # <group-key> <priority-delta>
	g=$1
	oldIFS=$IFS
	IFS='
'
	set -f
	fails=""
	for nm in $(group_wg_names "$g"); do
		[ -n "$nm" ] || continue
		reorder_one "$nm" "$2" || fails="$fails, $nm"
	done
	set +f
	IFS=$oldIFS
	[ -z "$fails" ] || notify "Failed to reorder:${fails#,}"
}

group_manual() { # <group-key>
	g=$1
	oldIFS=$IFS
	IFS='
'
	set -f
	fails=""
	for nm in $(group_wg_names "$g"); do
		[ -n "$nm" ] || continue
		OK=$(manual_one "$nm")
		[ -z "$OK" ] || fails="$fails, $nm"
	done
	set +f
	IFS=$oldIFS
	if [ -z "$fails" ]; then
		notify "$g is now manual"
	else
		notify "Failed for:${fails#,}"
	fi
}

# One row's worth of behaviour, for top-level rows and expanded group
# members alike — the same profile row means the same thing in both places.
# <act> is active_tunnels' output, needed only by "toggle".
row_action() { # <action> <arg> <rc> <act>
	action=$1 arg=$2 rc=$3 act=$4
	case "$action" in
	protect | disarm | unsecure)
		if OUT=$(sudo mango-vpnguard "$action" 2>&1); then
			notify "$(mango-vpnguard status)"
		else
			notify "$OUT"
		fi
		;;
	portal-open)
		xdg-open http://neverssl.com >/dev/null 2>&1 &
		;;
	# Not yet in vpnguard's guarded list: opt in at a starting priority, then
	# dial it in the same step. Plain nmcli, no sudo, for the opt-in half —
	# opting a profile in or out of the chain is a userspace decision.
	guard)
		if nmcli connection modify "$arg" connection.autoconnect-priority 10 >/dev/null 2>&1; then
			if OUT=$(printf 'CONN=%s\n' "$arg" | sudo mango-vpnguard up 2>&1); then
				notify "$(mango-vpnguard status)"
			else
				notify "$OUT"
			fi
		else
			notify "Failed to add $arg"
		fi
		;;
	up)
		case "$rc" in
		10) reorder_one "$arg" 10 || notify "Failed to reorder $arg" ;; # Shift+Enter
		11) reorder_one "$arg" -10 || notify "Failed to reorder $arg" ;; # Alt+Enter
		12) # Ctrl+Enter: make manual
			OK=$(manual_one "$arg")
			if [ -z "$OK" ]; then
				notify "$arg is now manual"
			else
				notify "Failed to make $arg manual: $OK"
			fi
			;;
		*) # plain Enter: dial it
			if OUT=$(printf 'CONN=%s\n' "$arg" | sudo mango-vpnguard up 2>&1); then
				notify "$(mango-vpnguard status)"
			else
				notify "$OUT"
			fi
			;;
		esac
		;;
	toggle)
		if printf '%s\n' "$act" | cut -f2 | grep -qxF "$arg"; then
			OUT=$(nmcli connection down uuid "$arg" 2>&1) || notify "$OUT"
		else
			OUT=$(nmcli connection up uuid "$arg" 2>&1) || notify "$OUT"
		fi
		# No poke needed: `nmcli connection up/down` above is itself a real NM
		# event — net.rs's `nmcli monitor` child already regrades from it
		# (IRONBAR.md T3), the same as every other join/disconnect action.
		;;
	settings)
		nm-connection-editor &
		;;
	esac
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
	STATE=$(cat "$GUARD_STATE_FILE" 2>/dev/null || echo unknown)
	ACTIONS=$(guard_action_rows "$STATE")

	# One rofi call in a loop. Tab on a group row toggles its expansion and
	# redraws the same list; every other accepted key acts once and returns.
	# -selected-row puts the cursor back on the toggled header, so the
	# expand feels in-place even though rofi restarts. EXPANDED is a
	# TAB-separated list of open group keys (keys can contain spaces).
	EXPANDED="" SEL=0
	while :; do
		PROFILES=$(profile_rows "$ACT" "$EXPANDED")

		# One flat table for the whole picker: action<TAB>arg<TAB>label.
		# Guard actions first — the most likely pick sits on row 0, already
		# selected — then the rule, the profiles, and settings.
		ROWS=$(printf '%s\n\t\t%s\n%s\nsettings\t\t▸  Connection settings…' "$ACTIONS" "$RULE" "$PROFILES")

		if printf '%s\n' "$ROWS" | cut -f1 | grep -qx group; then
			MESG="Enter connects  ·  Tab expands a group"
		elif printf '%s\n' "$ROWS" | cut -f1 | grep -qx up; then
			MESG="Enter connects"
		else
			MESG="Enter selects"
		fi
		# The reorder chords work on any chain ("up") row but are only
		# advertised while a group is open — the closed picker stays a
		# two-gesture affair.
		[ -z "$EXPANDED" ] || MESG="$MESG  ·  Shift/Alt+Enter reorders  ·  Ctrl+Enter makes it manual"

		# kb-accept-alt/-custom/-custom-alt have to be unbound before
		# kb-custom-N can take Shift/Alt/Ctrl+Return (same as
		# docker-menu.sh), or rofi refuses the rebind outright ("Binding
		# `…` is already bound"). kb-element-next holds Tab by default and
		# needs the same clearing before kb-custom-4 can take it. Chords
		# act on "up" rows only, Tab on "group" rows only, see below.
		IDX=$(printf '%s\n' "$ROWS" | cut -f3- |
			rofi -dmenu -format i -markup-rows -p "VPN" \
				-mesg "$MESG" -selected-row "$SEL" \
				-theme-str 'window { width: 780px; } listview { spacing: 5px; } element { padding: 9px 8px; }' \
				-kb-element-next "" \
				-kb-accept-alt "" -kb-accept-custom "" -kb-accept-custom-alt "" \
				-kb-custom-1 "Shift+Return" -kb-custom-2 "Alt+Return" \
				-kb-custom-3 "Control+Return" -kb-custom-4 "Tab")
		RC=$?
		[ "$RC" = 0 ] || [ "$RC" = 10 ] || [ "$RC" = 11 ] || [ "$RC" = 12 ] || [ "$RC" = 13 ] || return
		case "$IDX" in '' | *[!0-9]*) return ;; esac

		ROW=$(printf '%s\n' "$ROWS" | sed -n "$((IDX + 1))p")
		ACTION=$(printf '%s' "$ROW" | cut -f1)
		ARG=$(printf '%s' "$ROW" | cut -f2)

		if [ "$RC" = 13 ]; then # Tab: toggle group expansion, stay open
			[ "$ACTION" = group ] || {
				SEL=$IDX
				continue
			}
			case "$TAB$EXPANDED$TAB" in
			*"$TAB$ARG$TAB"*)
				EXPANDED=$(printf '%s' "$EXPANDED" | tr '\t' '\n' |
					grep -vxF "$ARG" | paste -sd "$TAB" -)
				;;
			*) EXPANDED="${EXPANDED:+$EXPANDED$TAB}$ARG" ;;
			esac
			# The header's index is stable: the toggle only adds or
			# removes rows below it, so the cursor lands back on it.
			SEL=$IDX
			continue
		fi

		[ -n "$ACTION" ] || return # status/section row, Enter does nothing
		case "$RC" in
		10 | 11 | 12) [ "$ACTION" = up ] || return ;;
		esac
		if [ "$ACTION" = group ]; then # Enter on a header dials the best member
			group_connect "$ARG"
			return
		fi
		row_action "$ACTION" "$ARG" "$RC" "$ACT"
		return
	done
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
	scan 1 shadow "$(R4 10.0.0.0/20 tun0 1000)
$(R4 10.0.0.0/16 wg0 50)"
	scan 1 identical "$(R4 10.0.0.0/20 tun0 1000)
$(R4 10.0.0.0/20 wg0 50)"
	# ordinary subnetting on one interface is not a conflict
	scan 0 - "$(R4 10.0.0.0/20 tun0 1000)
$(R4 10.0.0.0/16 tun0 1000)"
	scan 0 - "$(R4 10.0.0.0/17 tun0)
$(R4 10.1.0.0/17 wg0)"
	# a host route landing inside someone else's subnet
	scan 1 shadow "$(R4 10.0.0.5 wg0 50)
$(R4 10.0.0.0/24 tun0 1000)"
	# several defaults are how a VPN takes over — only a metric tie is ambiguous
	scan 0 - "$(R4 default wlo1 600)
$(R4 default wg0 50)"
	scan 1 tie "$(R4 default wlo1 600)
$(R4 default wg0 600)"
	# IPv6, including the /64-inside-/32 case and link-local being ignored
	scan 1 shadow "$(R6 fd00:10:1::/64 wg0 50)
$(R6 fd00:10::/32 tun0 1000)"
	scan 0 - "$(R6 fd00:10:1::/64 wg0)
$(R6 fd00:10:2::/64 tun0)"
	scan 0 - "$(R6 fe80::/64 wg0)
$(R6 fe80::/64 tun0)"
	# families must never cross-match despite identical leading bits
	scan 0 - "$(R4 10.0.0.0/16 tun0)
$(R6 ::/0 wg0)"

	printf '\n-- vpnguard action rows (actions only, no status prose) --\n'
	actionrow() { # expected row (grep pattern), state
		got=$(guard_action_rows "$2")
		if printf '%s\n' "$got" | grep -qF "$1"; then
			printf 'ok    action-row  %-24s -> %s\n' "$2" "$1"
		else
			printf 'FAIL  action-row  %-24s -> wanted %s, got: %s\n' "$2" "$1" "$got"
			fails=$((fails + 1))
		fi
	}
	actionrow 'disarm		▸  Turn protection off' 'vpn:ProtonVPN (DE317)'
	actionrow 'protect		▸  Try again' 'blocked'
	actionrow 'unsecure		▸  Go without protection' 'blocked'
	actionrow 'portal-open		▸  Open the sign-in page' 'portal'
	actionrow 'protect		▸  Turn protection on' 'unsecured'
	actionrow 'protect		▸  Turn protection on' 'unconfigured'
	# missing/garbage state must never offer "disarm" — that claims protection
	# is already on, which for an unknown state it almost certainly is not.
	nodisarm() { # state
		got=$(guard_action_rows "$1")
		if printf '%s\n' "$got" | cut -f1 | grep -qx disarm; then
			printf 'FAIL  action-row  %-24s -> offered disarm\n' "$1"
			fails=$((fails + 1))
		else
			printf 'ok    action-row  %-24s -> no disarm\n' "$1"
		fi
	}
	nodisarm ''
	nodisarm 'garbage'

	printf '\n-- vpnguard profile rows (NM-derived, one row per profile) --\n'
	VGT=$(mktemp -d)
	trap 'rm -rf "$VGT"' EXIT
	# `list` shape: priority<TAB>role<TAB>dev<TAB>name — no id, the NM
	# connection name (spaces and all) is the only identity. "Spare Tunnel"
	# is WireGuard but not in `list` at all — the case that used to produce
	# two rows (one "not guarded", one "Other VPNs") for the same profile.
	cat >"$VGT/mango-vpnguard" <<-'EOF'
		#!/bin/sh
		[ "$1" = list ] && printf '10\tdefault\twg_home_full\tHome Full\n0\talways\twg_home\tHome Gateway\n'
	EOF
	chmod +x "$VGT/mango-vpnguard"
	all_profiles() { # no real nmcli in a test — see its own definition
		printf 'uuid-1\twireguard\tHome Full\n'
		printf 'uuid-2\twireguard\tHome Gateway\n'
		printf 'uuid-3\twireguard\tSpare Tunnel\n'
		printf 'uuid-4\tvpn\tWork OpenVPN\n'
	}
	profrow() { # expected row (grep pattern)
		if printf '%s\n' "$GOT" | grep -qF "$1"; then
			printf 'ok    profile-row  %s\n' "$1"
		else
			printf 'FAIL  profile-row  wanted %s, got: %s\n' "$1" "$GOT"
			fails=$((fails + 1))
		fi
	}
	GOT=$(VG_BIN="$VGT/mango-vpnguard" profile_rows 'wireguard	uuid-1	wg_home_full	Home Full')
	rm -rf "$VGT"
	profrow 'up	Home Full	<tt><span alpha="65%">●</span></tt>  Home Full<span alpha="55%">  WireGuard · auto #1 · connected · wg_home_full</span>'
	profrow 'up	Home Gateway	<tt><span alpha="65%">○</span></tt>  Home Gateway<span alpha="55%">  WireGuard · always</span>'
	profrow 'guard	Spare Tunnel	<tt><span alpha="65%">○</span></tt>  Spare Tunnel<span alpha="55%">  WireGuard · manual</span>'
	profrow 'toggle	uuid-4	<tt><span alpha="65%">○</span></tt>  Work OpenVPN<span alpha="55%">  OpenVPN · manual</span>'
	# the bug this replaced the old two-source lookup for: no argument
	# (connection name or uuid) may appear on more than one row.
	dups=$(printf '%s\n' "$GOT" | cut -f2 | sort | uniq -d)
	if [ -z "$dups" ]; then
		printf 'ok    profile-row  no argument duplicated\n'
	else
		printf 'FAIL  profile-row  duplicated argument(s): %s\n' "$(printf '%s' "$dups" | tr '\n' ' ')"
		fails=$((fails + 1))
	fi

	printf '\n-- vpnguard profile rows (grouping by name prefix) --\n'
	VGT2=$(mktemp -d)
	cat >"$VGT2/mango-vpnguard" <<-'EOF'
		#!/bin/sh
		[ "$1" = list ] || exit 0
	EOF
	chmod +x "$VGT2/mango-vpnguard"
	all_profiles() { # three "ProtonVPN (…)" servers plus one unrelated singleton
		printf 'uuid-a\twireguard\tProtonVPN (DE317)\n'
		printf 'uuid-b\twireguard\tProtonVPN (MU31)\n'
		printf 'uuid-c\tvpn\tProtonVPN (Mauritius)\n'
		printf 'uuid-d\twireguard\tarrakis\n'
	}
	GROUPGOT=$(VG_BIN="$VGT2/mango-vpnguard" profile_rows '')
	EXPGOT=$(VG_BIN="$VGT2/mango-vpnguard" profile_rows '' 'ProtonVPN')
	rm -rf "$VGT2"

	grpcheck() { # description, grep pattern against $GROUPGOT
		if printf '%s\n' "$GROUPGOT" | grep -qF "$2"; then
			printf 'ok    group-row    %s\n' "$1"
		else
			printf 'FAIL  group-row    %s -> got: %s\n' "$1" "$GROUPGOT"
			fails=$((fails + 1))
		fi
	}
	grpcheck 'three Proton servers collapse to one row' 'group	ProtonVPN	'
	grpcheck 'the collapsed row states its member count' '3 servers'
	grpcheck 'the collapsed row shows a closed chevron' '▸ ProtonVPN'
	grpcheck 'an ungrouped profile still renders on its own' 'guard	arrakis	'
	if printf '%s\n' "$GROUPGOT" | grep -qF 'ProtonVPN (DE317)'; then
		printf 'FAIL  group-row    a grouped member must not also get its own row\n'
		fails=$((fails + 1))
	else
		printf 'ok    group-row    grouped members do not also get their own row\n'
	fi

	expcheck() { # description, grep pattern against $EXPGOT
		if printf '%s\n' "$EXPGOT" | grep -qF "$2"; then
			printf 'ok    group-expand %s\n' "$1"
		else
			printf 'FAIL  group-expand %s -> got: %s\n' "$1" "$EXPGOT"
			fails=$((fails + 1))
		fi
	}
	expcheck 'an expanded group lists its members' 'guard	ProtonVPN (DE317)	'
	expcheck '  and every member, not just one' 'guard	ProtonVPN (MU31)	'
	expcheck '  including a non-WireGuard one' 'toggle	uuid-c	'
	hn=$(printf '%s\n' "$EXPGOT" | grep -cF 'group	ProtonVPN	')
	if [ "$hn" = 1 ] && printf '%s\n' "$EXPGOT" | grep -F 'group	ProtonVPN	' | grep -qF '▾'; then
		printf 'ok    group-expand the group keeps one open (▾) header row\n'
	else
		printf 'FAIL  group-expand want one ▾ header row, got: %s\n' "$EXPGOT"
		fails=$((fails + 1))
	fi
	hline=$(printf '%s\n' "$EXPGOT" | grep -nF 'group	ProtonVPN	' | cut -d: -f1 | head -1)
	members=$(printf '%s\n' "$EXPGOT" | sed -n "$((hline + 1)),$((hline + 3))p" | cut -f2)
	if [ "$members" = "$(printf 'ProtonVPN (DE317)\nProtonVPN (MU31)\nuuid-c')" ]; then
		printf 'ok    group-expand members sit directly under the header, in order\n'
	else
		printf 'FAIL  group-expand member rows out of place: %s\n' "$(printf '%s' "$members" | tr '\n' ' ')"
		fails=$((fails + 1))
	fi
	en=$(printf '%s\n' "$EXPGOT" | grep -c .)
	if [ "$en" = 5 ]; then
		printf 'ok    group-expand full list: header + 3 members + arrakis\n'
	else
		printf 'FAIL  group-expand want 5 rows, got %d\n' "$en"
		fails=$((fails + 1))
	fi

	# Row-table invariant the old index maths used to need proving separately:
	# cutting the label column and cutting the action column from the same
	# table must stay in lock-step, row for row — that's what the flat
	# action<TAB>arg<TAB>label table in sec_click depends on.
	printf '\n-- row-table invariant --\n'
	rowinv() { # <set name> <rows>
		NROWS=$(printf '%s\n' "$2" | grep -c .)
		i=1
		rowfails=0
		while [ "$i" -le "$NROWS" ]; do
			row=$(printf '%s\n' "$2" | sed -n "${i}p")
			act=$(printf '%s' "$row" | cut -f1)
			arg=$(printf '%s' "$row" | cut -f2)
			lbl=$(printf '%s' "$row" | cut -f3-)
			[ "$row" = "$act$TAB$arg$TAB$lbl" ] || {
				printf 'FAIL  row-table  %s row %d: label/action cut out of step with the full row\n' "$1" "$i"
				fails=$((fails + 1))
				rowfails=$((rowfails + 1))
			}
			i=$((i + 1))
		done
		[ "$rowfails" -gt 0 ] || printf 'ok    row-table  %s: %d row(s) in lock-step\n' "$1" "$NROWS"
	}
	rowinv 'guard-actions' "$(guard_action_rows blocked)"
	rowinv 'expanded-group' "$EXPGOT"

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
