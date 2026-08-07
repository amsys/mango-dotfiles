#!/bin/bash
# One waybar bar per monitor, each drawing its own tags.
#
# mango gives every monitor an independent set of nine tags, but waybar has no
# way to tell a custom module which output its bar is on — no --output flag in
# 0.15, no environment, no format placeholder. So the monitor name has to reach
# workspace.sh through its exec line, which means one bar object per output,
# which means the config has to be generated: the monitor set is per-machine and
# changes when you dock.
#
# waybar's own multi-bar support does the rest. A config may be an array of bar
# objects, each with its own `output`, and `include` merges config.jsonc
# underneath — recursively, so a bar overrides one `exec` string per pill and
# inherits `signal`, `escape`, `on-click` and the rest from the tracked file.
# That is deliberate: config.jsonc stays the only place the pills are defined,
# and a bare `waybar` still gets working (single-monitor) pills.
#
# Nothing here touches on-click. mango sets selmon from the cursor on every
# button press (mango.c, WL_POINTER_BUTTON_STATE_PRESSED) and on motion under
# sloppyfocus, so by the time waybar shells out `mmsg dispatch view,N,0` the
# focused monitor is already the one the bar is on — same for the scroll binds.
# Only the rendering was ever monitor-blind.
#
#   bars.sh        generate, launch waybar, then follow monitor hotplug
#   bars.sh test   assert the generator against canned IPC output
set -u

CFG="${XDG_RUNTIME_DIR:-/tmp}/waybar-bars.json"
SHARED="$HOME/.config/waybar/config.jsonc"
WS="$HOME/.config/waybar/scripts/workspace.sh"

# `include` is resolved by waybar, not by a shell, so the path is absolute — a
# relative one would resolve against mango's cwd, and a missing include is a
# crash rather than a warning.
#
# The last element is the catch-all: `!name` exclusions followed by `*` (see the
# `output` key in waybar(5)) means any monitor NOT in the generated set still
# gets a bar the instant it is plugged in, falling back to first-monitor tags
# until the hotplug watcher below regenerates. Without it a new output would
# come up with no bar at all, which is worse than the bug this fixes.
bars() { # all-monitors JSON on stdin -> waybar bar array on stdout
	jq --arg cfg "$SHARED" --arg ws "$WS" '
		[ .monitors[].name ] as $names |
		[ $names[] | . as $m |
			{ output: $m, include: [$cfg] } +
			([ range(1; 10) | tostring |
				{ ("custom/ws#" + .): { exec: "\($ws) \(.) \($m)" } } ] | add) ]
		+ [ { output: ([ $names[] | "!" + . ] + ["*"]), include: [$cfg] } ]'
}

# Atomic: waybar re-reads this path on every SIGUSR2, including the one
# switchwall.sh sends after a palette change, so it must never be seen half
# written or missing. Requires more than one element — the catch-all alone means
# the monitor query came back empty.
gen() {
	local out
	out=$(mmsg get all-monitors 2> /dev/null | bars 2> /dev/null) || return 1
	[ "$(printf '%s' "$out" | jq 'length' 2> /dev/null || echo 0)" -gt 1 ] || return 1
	printf '%s\n' "$out" > "$CFG.tmp" && mv -f "$CFG.tmp" "$CFG"
}

# GTK3's tooltip hover delay is a hardcoded 500ms with no setting to shorten
# it (see README) — install-config.sh compiles an LD_PRELOAD shim for this.
# Empty SHIM if it hasn't been built yet, so a fresh checkout still starts
# waybar normally.
SHIM="$HOME/.local/lib/mango/fast-tooltips.so"
[ -f "$SHIM" ] || SHIM=""
# exec, not a plain call: backgrounding a function forks a subshell around it,
# and without exec that subshell just sits as waybar's parent — $! then names
# the subshell, and kill/wait downstream stop hitting waybar at all.
bar() { exec env LD_PRELOAD="$SHIM" waybar "$@"; }

# ---------------------------------------------------------------- selftest

if [ "${1:-}" = test ]; then
	MONS='{"monitors":[{"name":"eDP-1","active":false},{"name":"DP-1","active":true}]}'
	OUT=$(printf '%s\n' "$MONS" | bars) || { echo "bars failed"; exit 1; }
	q() { printf '%s' "$OUT" | jq -r "$1"; }

	# Two monitors -> two bars plus the catch-all.
	[ "$(q 'length')" = 3 ] || { echo "bar count wrong"; exit 1; }
	[ "$(q '.[1].output')" = DP-1 ] || { echo "output key wrong"; exit 1; }
	[ "$(q '.[0].include[0]')" = "$SHARED" ] || { echo "include wrong"; exit 1; }
	# Nine pills and nothing else: any other key here would shadow config.jsonc.
	[ "$(q '.[0] | keys | length')" = 11 ] || { echo "bar key count wrong"; exit 1; }
	[ "$(q '[.[0] | keys[] | select(startswith("custom/ws#"))] | length')" = 9 ] \
		|| { echo "pill count wrong"; exit 1; }
	# Exec only. Overriding on-click here would fork the click semantics away
	# from config.jsonc, which is where they are documented.
	[ "$(q '.[0]["custom/ws#3"] | keys | join(",")')" = exec ] || { echo "pill must override exec only"; exit 1; }
	# The whole point: the same tag index carries a different monitor per bar.
	[ "$(q '.[0]["custom/ws#3"].exec')" = "$WS 3 eDP-1" ] || { echo "exec wrong"; exit 1; }
	[ "$(q '.[1]["custom/ws#3"].exec')" = "$WS 3 DP-1" ] || { echo "exec monitor wrong"; exit 1; }
	# The catch-all excludes every known monitor and claims the rest, and names
	# no monitor of its own — a hot-plugged output must not inherit DP-1's tags.
	[ "$(q '.[2].output | join(",")')" = '!eDP-1,!DP-1,*' ] || { echo "catch-all output wrong"; exit 1; }
	[ "$(q '.[2] | keys | join(",")')" = include,output ] || { echo "catch-all must override nothing"; exit 1; }
	# A single monitor still produces an array, or waybar draws no bar at all.
	[ "$(printf '{"monitors":[{"name":"eDP-1"}]}' | bars | jq 'length')" = 2 ] || { echo "single monitor wrong"; exit 1; }
	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- launch

# exec-once fires at compositor start, so the IPC socket may not be answering
# yet — same race config.conf's arch-update line waits out. Losing it would cost
# the whole session its per-monitor bars, so retry before giving up.
for _ in 1 2 3 4 5; do
	gen && break
	sleep 0.5
done

# No IPC, no generated config: the plain bar. That costs the tag row its
# per-monitor accuracy, not the whole bar.
[ -s "$CFG" ] || bar

# This script owns waybar from here on: the trap takes it, and the watch
# producers, down with us rather than orphaning them on every restart — a
# `mmsg watch` left in a pipeline is exactly the leak watch.sh exists to avoid.
trap 'pkill -P $$ > /dev/null 2>&1' EXIT INT TERM

bar -c "$CFG" &
BAR=$!

# Hotplug: regenerate, then RESTART waybar. Not SIGUSR2 — waybar's reload
# re-reads the config but does not rebuild the bar list from it, so a bar for a
# newly connected output never appears and the bars it already had can go with
# it. Measured, not assumed. The restart is safe: an already-running tray applet
# reconnects (config.conf's arch-update note is about its *startup*, when no
# StatusNotifierHost is listening yet — a live one survives).
#
# all-monitors also carries focus, tag and keymode state, so it fires constantly
# under sloppyfocus. One long-lived jq reduces the stream to a sorted name list
# and only a real change to the set restarts anything. The loop reads from a
# process substitution rather than a pipeline so it stays in this shell and can
# still manage $BAR.
prev=$(mmsg get all-monitors 2> /dev/null | jq -r '[.monitors[].name] | sort | join(" ")')

while IFS= read -r cur; do
	[ "$cur" = "$prev" ] && continue
	prev=$cur
	gen || continue
	kill "$BAR" 2> /dev/null
	wait "$BAR" 2> /dev/null
	bar -c "$CFG" &
	BAR=$!
done < <(mmsg watch all-monitors 2> /dev/null |
	jq -r --unbuffered '[.monitors[].name] | sort | join(" ")' 2> /dev/null)

wait
