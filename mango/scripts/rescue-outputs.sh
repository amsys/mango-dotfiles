#!/usr/bin/env bash
# Detects and recovers from mango's "frozen desktop" state: every output's
# geometry zeroed (mango.c:7176) while the outputs are still tracked and
# re-enablable via `mmsg dispatch enable_monitor` — no compositor restart, no
# lost applications. Confirmed live 2026-08-26 (see IRONBAR.md and
# ~/.claude/plans/read-home-martin-claude-plans-find-out-w-precious-torvalds.md
# for the source analysis this script is built on).
#
# `wlopm --off` (DPMS) does NOT trigger this: an only_sleep monitor keeps its
# real geometry (mango.c:7168-7170), so this script never touches it.
#
#   rescue-outputs.sh        one-shot: check now, rescue if frozen. Bound to
#                             SUPER+SHIFT+o and safe from a bare TTY.
#   rescue-outputs.sh watch  poll loop for mango-outputs.service.
#   rescue-outputs.sh test   assert the frozen/normal/DPMS JSON shapes
#                             classify correctly. No compositor needed.
set -uo pipefail

POLL_INTERVAL=5   # seconds between polls in watch mode
CONFIRM_SAMPLES=2 # consecutive frozen samples before acting (~10s, well
                   # inside the 100s freeze-to-crash window observed live)
COOLDOWN=60        # seconds to wait after a rescue attempt before trying again

log() { printf 'rescue-outputs.sh: %s\n' "$*" >&2; }

notify() { # urgency summary body
	command -v notify-send >/dev/null 2>&1 && notify-send -a mango -u "$1" "$2" "$3" || true
}

# ---------------------------------------------------------------- primitives

# mango exports MANGO_INSTANCE_SIGNATURE to every child it forks, but the
# path is per-mango-pid: it goes stale the moment session.sh restarts the
# compositor, and it is absent entirely from a bare TTY — the one place this
# script most needs to work (input still responds during a freeze; a TTY
# escape is the fallback if the keybind can't reach mango either). Probe
# candidates by actually talking to them instead of trusting the filename.
resolve_socket() {
	local candidate
	if [[ -n ${MANGO_INSTANCE_SIGNATURE:-} ]] &&
		MANGO_INSTANCE_SIGNATURE=$MANGO_INSTANCE_SIGNATURE mmsg get version >/dev/null 2>&1; then
		return 0
	fi
	while IFS= read -r candidate; do
		if MANGO_INSTANCE_SIGNATURE=$candidate mmsg get version >/dev/null 2>&1; then
			export MANGO_INSTANCE_SIGNATURE=$candidate
			return 0
		fi
	done < <(find "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" -maxdepth 1 -name 'mango-*.sock' -printf '%T@ %p\n' 2>/dev/null |
		sort -rn | cut -d' ' -f2-)
	return 1
}

monitors_json() { mmsg get all-monitors 2>/dev/null; }

# Frozen: every monitor's width/height zeroed (mango.c:7176). Requiring
# EVERY monitor, not any, keeps a deliberately single-disabled external
# display from being fought by watch mode.
is_frozen() {
	jq -e '.monitors | length > 0 and all(.[]; .width == 0 and .height == 0)' \
		>/dev/null 2>&1 <<<"$1"
}

# enable_monitor (bind_define.h) only flips `enabled`; it never re-applies
# monitorrule geometry — only createmon (hotplug) does. So mango rebuilds a
# re-enabled output's position with an auto-layout that appends each newly
# (re-)enabled output to the right of whatever is already placed: enabling
# in the wrong order silently swaps a multi-monitor layout. Confirmed live
# 2026-08-26 — enabling in `mmsg get all-monitors` list order (DP-1 before
# eDP-1) put DP-1 at x:0 and eDP-1 at x:1920, the reverse of the configured
# layout, which reversed which physical screen the mouse crossed into.
#
# Fix: read monitorrule= lines (config.conf + local.conf, config.conf's
# rule search order) that carry both name: and an explicit x:, and enable
# those first, ascending by x, so the auto-layout reconstructs the
# configured left-to-right order; enable everything else afterward.
# ponytail: matches only a literal, single, fully-anchored name: value
# (`^eDP-1$` -> `eDP-1`) — a monitorrule using a bare substring or an
# alternation falls through to "no anchor" and keeps list order. Revisit if
# a monitorrule here ever needs one.
anchor_order() { # reads monitorrule= lines on stdin, prints names by ascending x
	grep '^monitorrule=' | sed 's/^monitorrule=//' |
		awk -F, '{
			x = ""; nm = "";
			for (i = 1; i <= NF; i++) {
				if ($i ~ /^name:\^[^$]*\$$/) { nm = $i; sub(/^name:\^/, "", nm); sub(/\$$/, "", nm) }
				if ($i ~ /^x:/) { x = $i; sub(/^x:/, "", x) }
			}
			if (nm != "" && x != "") print x, nm
		}' | sort -n | awk '{print $2}'
}

configured_anchor_order() {
	{
		cat "$HOME/.config/mango/config.conf" 2>/dev/null
		cat "$HOME/.config/mango/local.conf" 2>/dev/null
	} | anchor_order
}

# match_monitor_spec (common/util.c) is an unanchored PCRE2 search, so
# "DP-1" also matches "eDP-1" — anchor every name we dispatch. A
# non-matching dispatch is a silent no-op (bind_define.h), so this never
# trusts the dispatch call itself; the caller re-reads state to confirm.
rescue() {
	local json=$1 name ordered=()
	log "frozen, monitors: $json"

	while IFS= read -r name; do
		[[ -n $name ]] && ordered+=("$name")
	done < <(configured_anchor_order)
	while read -r name; do
		[[ -n $name ]] || continue
		printf '%s\n' "${ordered[@]:-}" | grep -qx "$name" || ordered+=("$name")
	done < <(jq -r '.monitors[].name' <<<"$json")

	for name in "${ordered[@]}"; do
		mmsg dispatch "enable_monitor,^${name}\$" >/dev/null 2>&1 || true
	done

	local after
	after=$(monitors_json)
	if jq -e '.monitors | any(.[]; .width > 0)' >/dev/null 2>&1 <<<"$after"; then
		log "recovered: $after"
		notify normal "Display rescue" "Outputs re-enabled."
		return 0
	fi
	log "rescue attempt failed, still frozen: $after"
	notify critical "Display rescue" "Outputs still frozen — enable_monitor did not recover them."
	return 1
}

# ---------------------------------------------------------------------- modes

run_once() {
	resolve_socket || {
		log "no reachable mango IPC socket"
		return 1
	}
	local json
	json=$(monitors_json)
	if [[ -z $json ]]; then
		log "mmsg get all-monitors returned nothing"
		return 1
	fi
	if is_frozen "$json"; then
		rescue "$json"
	else
		log "not frozen"
	fi
}

run_watch() {
	local frozen_count=0 cooldown_until=0 now json
	while :; do
		sleep "$POLL_INTERVAL"
		now=$(date +%s)
		((now < cooldown_until)) && continue

		resolve_socket || {
			frozen_count=0
			continue
		}
		json=$(monitors_json)
		if [[ -z $json ]]; then
			frozen_count=0
			continue
		fi

		if is_frozen "$json"; then
			((++frozen_count))
			if ((frozen_count >= CONFIRM_SAMPLES)); then
				rescue "$json" || true
				frozen_count=0
				cooldown_until=$((now + COOLDOWN))
			fi
		else
			frozen_count=0
		fi
	done
}

run_test() {
	local normal frozen dpms

	normal='{"monitors":[{"name":"eDP-1","width":1920,"height":1080}]}'
	frozen='{"monitors":[{"name":"eDP-1","width":0,"height":0},{"name":"DP-1","width":0,"height":0}]}'
	# only_sleep (DPMS-off) preserves real geometry — same shape as normal,
	# and this is the case that must NOT be classified frozen.
	dpms='{"monitors":[{"name":"eDP-1","width":1920,"height":1080}]}'

	is_frozen "$normal" && { echo "FAIL: normal classified frozen"; exit 1; }
	is_frozen "$frozen" || { echo "FAIL: frozen state not detected"; exit 1; }
	is_frozen "$dpms" && { echo "FAIL: DPMS-off classified frozen"; exit 1; }
	is_frozen '{"monitors":[]}' && { echo "FAIL: empty monitor list classified frozen"; exit 1; }

	local order
	order=$(printf 'monitorrule=name:^DP-1$,x:1920,width:1920,height:1080\nmonitorrule=name:^eDP-1$,x:0,width:1920,height:1080\n' | anchor_order | tr '\n' ' ')
	[[ $order == "eDP-1 DP-1 " ]] || { echo "FAIL: anchor_order not ascending by x, got: $order"; exit 1; }

	order=$(printf '' | anchor_order | tr '\n' ' ')
	[[ -z $order ]] || { echo "FAIL: anchor_order on no rules should be empty, got: $order"; exit 1; }

	echo "ok"
}

case "${1:-}" in
test) run_test ;;
watch) run_watch ;;
*) run_once ;;
esac
