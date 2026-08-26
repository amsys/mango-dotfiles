#!/usr/bin/env bash
# Bar launcher — exec-once'd from mango/config.conf. Replaces waybar/bars.sh
# at the T8 cutover (IRONBAR.md). mango-bard itself runs as its own systemd
# user unit (mango-bard.service), started independently — this script only
# generates the bar config from the live monitor list and execs ironbar.
set -euo pipefail

CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/ironbar/config.json"
mango-bard gen-config --out "$CONFIG"

# GTK3's hardcoded 500ms hover-tooltip delay (see install-config.sh's own
# comment on the shim) — carried over from bars.sh's `env LD_PRELOAD=...
# waybar`. Skipped, not fatal, if install-config.sh couldn't build it (no
# cc/glib-2.0 dev headers): the bar still runs, just with the stock delay.
SHIM="$HOME/.local/lib/mango/fast-tooltips.so"
[ -f "$SHIM" ] && export LD_PRELOAD="$SHIM"

# Stale-state guard: a login into a runtime dir left over from a crashed
# prior session (compositor died mid-handoff) finds a still-alive ironbar
# and its IPC socket already there. A fresh ironbar tries to take over that
# socket and can hang forever mid-handshake — confirmed live (2026-08-26):
# `g_application_run` parked in a blocking syscall, no bar surface, no IPC
# listener, ironbar ping failing (see IRONBAR.md). Clear any leftover
# instance and its socket before every launch, not just after a crash —
# cheap and always correct even when nothing was left behind.
SOCK="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/ironbar-ipc.sock"
pkill -x ironbar 2>/dev/null || true
for _ in 1 2 3 4 5; do pgrep -x ironbar >/dev/null || break; sleep 0.2; done
rm -f "$SOCK"

# Launch and confirm the bar actually came up (the guard above handles the
# known cause; this catches anything else). One retry — a bar that fails
# twice in a row has a different problem and should stay visibly broken
# rather than loop.
for attempt in 1 2; do
	ironbar &
	pid=$!
	for _ in $(seq 1 20); do
		ironbar ping >/dev/null 2>&1 && { wait "$pid"; exit $?; }
		sleep 0.5
	done
	echo "start.sh: ironbar did not answer ping within 10s (attempt $attempt), retrying" >&2
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	rm -f "$SOCK"
done
echo "start.sh: ironbar failed to start after 2 attempts, giving up" >&2
exit 1
