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
CONF="${MANGO_POWERMODE_CONF:-$HOME/.config/mango/powermode.conf}"
RUN="${XDG_RUNTIME_DIR:-/tmp}"

MODE=full     # re-resolved by resolve_pm() before every gen(); full is the safe default
ECO_JSON='{}' # per-module interval overrides, applied only when MODE=eco

# Reads mango/scripts/powermode.sh's current mode and mango/powermode.conf's
# PM_ECO_INTERVAL_* — called fresh at the top of every gen(), not just once at
# startup, so a mode switch or a config edit takes effect on the next restart
# without needing bars.sh itself relaunched.
resolve_pm() {
	MODE=full
	[ -r "$RUN/mango-powermode" ] && { IFS= read -r MODE < "$RUN/mango-powermode"; } 2> /dev/null
	: "${MODE:=full}"

	# shellcheck disable=SC1090  # the user's own tracked config, same as powermode.sh
	[ -r "$CONF" ] && . "$CONF"
	# custom/netsec is deliberately never in this object, eco or not: it's the
	# leak monitor, and a lock that can be five minutes stale defeats its own
	# purpose. It keeps config.jsonc's 60s in both modes; only its CSS animation
	# (see the .eco.open/.eco.portal rule in the style template) responds to
	# eco.
	ECO_JSON=$(jq -n \
		--argjson cpu "${PM_ECO_INTERVAL_CPU:-15}" \
		--argjson mem "${PM_ECO_INTERVAL_MEM:-30}" \
		--argjson dk "${PM_ECO_INTERVAL_DOCKER:-60}" \
		--argjson wifi "${PM_ECO_INTERVAL_WIFI:-120}" \
		--argjson eth "${PM_ECO_INTERVAL_ETH:-300}" \
		--argjson bat "${PM_ECO_INTERVAL_BATTERY:-60}" \
		'{"custom/cpu": {interval: $cpu}, "custom/memory": {interval: $mem},
		  "custom/docker": {interval: $dk}, "custom/wifi": {interval: $wifi},
		  "custom/eth": {interval: $eth},
		  "custom/battery": {interval: $bat}}')
}

# `include` is resolved by waybar, not by a shell, so the path is absolute — a
# relative one would resolve against mango's cwd, and a missing include is a
# crash rather than a warning.
#
# The last element is the catch-all: `!name` exclusions followed by `*` (see the
# `output` key in waybar(5)) means any monitor NOT in the generated set still
# gets a bar the instant it is plugged in, falling back to first-monitor tags
# until the hotplug watcher below regenerates. Without it a new output would
# come up with no bar at all, which is worse than the bug this fixes. It never
# gets the eco interval override below — a freshly hot-plugged monitor polling
# at full speed until the next regen is a smaller cost than the added branch.
bars() { # all-monitors JSON on stdin -> waybar bar array on stdout
	jq --arg cfg "$SHARED" --arg ws "$WS" --arg mode "${MODE:-full}" --argjson eco "${ECO_JSON:-\{\}}" '
		(if $mode == "eco" then $eco else {} end) as $ov |
		[ .monitors[].name ] as $names |
		[ $names[] | . as $m |
			({ output: $m, include: [$cfg] } +
				([ range(1; 10) | tostring |
					{ ("custom/ws#" + .): { exec: "\($ws) \(.) \($m)" } } ] | add)) * $ov ]
		+ [ { output: ([ $names[] | "!" + . ] + ["*"]), include: [$cfg] } ]'
}

# Atomic: waybar re-reads this path on every SIGUSR2, including the one
# switchwall.sh sends after a palette change, so it must never be seen half
# written or missing. Requires more than one element — the catch-all alone means
# the monitor query came back empty.
gen() {
	local out
	resolve_pm
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
#
# 9>&- closes the lock fd (opened below, in the launch section) on the way
# in: exec replaces the process image but keeps file descriptors open by
# default, so without this a launched waybar would sit on fd 9 and hold
# mango-bars.lock for as long as it runs. Proven live, on the *other* fd-9
# leak this script had (the hotplug watcher below, now closed the same way):
# killing just the controller left an orphaned child still holding the lock,
# and a fresh bars.sh's `flock -n 9` refused to start — silently, forever,
# until that orphan was found and killed by hand.
bar() { exec 9>&- env LD_PRELOAD="$SHIM" waybar "$@"; }

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

	# --- eco mode: every per-monitor bar gains the interval overrides ---
	MODE=eco
	ECO_JSON=$(jq -n '{"custom/cpu": {interval: 15}, "custom/battery": {interval: 60}}')
	OUT=$(printf '%s\n' "$MONS" | bars) || { echo "bars failed in eco mode"; exit 1; }
	q() { printf '%s' "$OUT" | jq -r "$1"; }
	[ "$(q '.[0]["custom/cpu"].interval')" = 15 ] || { echo "eco should override custom/cpu interval"; exit 1; }
	[ "$(q '.[1]["custom/battery"].interval')" = 60 ] || { echo "eco override should apply to every monitor's bar"; exit 1; }
	# config.jsonc's own exec/on-click for an overridden module must still come
	# through `include` untouched — this only ever adds an `interval` key.
	[ "$(q '.[0]["custom/ws#3"].exec')" = "$WS 3 eDP-1" ] || { echo "eco override must not disturb unrelated pills"; exit 1; }
	# the catch-all is deliberately never eco-overridden
	[ "$(q '.[2] | keys | join(",")')" = include,output ] || { echo "catch-all must stay override-free even in eco"; exit 1; }
	MODE=full
	ECO_JSON='{}'

	# --- full mode is unaffected even with an eco override loaded ---
	MODE=full
	ECO_JSON=$(jq -n '{"custom/cpu": {interval: 15}}')
	OUT=$(printf '%s\n' "$MONS" | bars) || { echo "bars failed in full mode"; exit 1; }
	printf '%s' "$OUT" | jq -e '.[0]["custom/cpu"]' > /dev/null 2>&1 \
		&& { echo "full mode must not apply the eco override even if one is loaded"; exit 1; }
	MODE=full
	ECO_JSON='{}'

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- launch

# One waybar, ever. mango's exec-once can in principle re-fire (a compositor
# restart re-runs config.conf without a fresh login), and a bars.sh killed
# with -9 leaves its EXIT trap unrun, orphaning waybar with nothing left to
# manage it — either way, a second bars.sh has to take over cleanly rather
# than draw a second bar on top of the first. flock is the mutex; holding it
# means nothing else is mid-launch, so an existing waybar is safe to reap.
exec 9> "$RUN/mango-bars.lock"
flock -n 9 || exit 0
pkill -x waybar 2> /dev/null
printf '%s' "$$" > "$RUN/mango-bars.pid" # read by powermode.sh to signal us on a mode change

# Trap installed before anything is launched below, including the degraded
# fallback: that fallback used to `exec` straight into waybar, which replaced
# this process *before* the trap existed — the pid file above then named
# waybar itself, so a later mode-change USR1 (see powermode.sh's bars_restart)
# would land on waybar's own SIGUSR1 (toggle visibility, hides the bar
# indefinitely) instead of on us. Backgrounding it below, every launch path
# through, keeps this pid file accurate for as long as bars.sh runs.
#
# This script owns waybar from here on: the trap takes it, and the watch
# producers, down with us rather than orphaning them on every restart — a
# `mmsg watch` left in a pipeline is exactly the leak watch.sh exists to avoid.
trap 'rm -f "$RUN/mango-bars.pid"; pkill -P $$ > /dev/null 2>&1' EXIT INT TERM

# exec-once fires at compositor start, so the IPC socket may not be answering
# yet — same race config.conf's arch-update line waits out. Losing it would cost
# the whole session its per-monitor bars, so retry before giving up.
for _ in 1 2 3 4 5; do
	gen && break
	sleep 0.5
done

# Anything named waybar that isn't the one we're about to manage — a manual
# launch, or a second bars.sh's fallback from before it took over. Measured
# live: a stray survives forever otherwise, since every restart/reload path
# below only ever touches $BAR.
reap_strays() { pgrep -x waybar 2> /dev/null | grep -vx "${BAR:-}" | xargs -r kill 2> /dev/null; }

if [ -s "$CFG" ]; then
	bar -c "$CFG" &
else
	# No IPC, no generated config: the plain bar. That costs the tag row its
	# per-monitor accuracy, not the whole bar. Backgrounded like the normal
	# path, not exec'd — see the trap comment above for why that matters.
	bar &
fi
BAR=$!

restart_bar() { # regenerate first; caller decides whether to
	reap_strays
	kill "$BAR" 2> /dev/null
	wait "$BAR" 2> /dev/null
	bar -c "$CFG" &
	BAR=$!
}

# Reload rather than restart: a mode switch changes nothing but a handful of
# module `interval`s inside bars that already exist, so there is no torn
# state to avoid by deferring — unlike the hotplug path below, where the bar
# *list* itself changes. SIGUSR2 alone doesn't cost the bar its brief absence
# a full kill+relaunch does, and by the time a mode switch signals us,
# powermode.sh has usually already dropped the CPU to eco — the coldest
# possible clock to cold-start waybar on.
reload_bar() {
	reap_strays
	kill -USR2 "$BAR" 2> /dev/null
}

# USR1, not HUP: measured live that `nohup`-launched (or otherwise
# HUP-preignoring) parents make HUP permanently untrappable here — bash
# refuses to install a trap for any signal that was already SIG_IGN when the
# shell started (see bash(1), SIGNALS), and there is no reliable way from
# inside this script to know whether whatever launched it did that. USR1 is
# never preignored by anything in this chain and isn't used by waybar's own
# SIGUSR2 reload, so there's nothing to collide with.
#
# Acted on directly, not deferred through a flag: a mode switch has no bar-list
# change to race against (see reload_bar above), so there is no torn state
# the old flag-and-poll dance was protecting against here — only the hotplug
# path below still needs that.
trap 'gen && reload_bar' USR1

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

while :; do
	rc=0
	IFS= read -r -t 2 cur <&3 || rc=$?
	if [ "$rc" = 0 ]; then
		if [ "$cur" != "$prev" ]; then
			prev=$cur
			gen && restart_bar
		fi
	elif [ "$rc" -le 128 ]; then
		break # a genuine EOF/error, not a timeout: mmsg watch is not coming back
	fi
	# Still degraded (launched without a generated config, or gen() has been
	# failing): retry every time this loop wakes rather than waiting for
	# another hotplug event to come along and fix it as a side effect.
	[ -s "$CFG" ] || { gen && restart_bar; }
# exec 9>&- first: this subshell is long-lived for as long as bars.sh runs,
# and without closing its inherited copy of the lock fd, killing only the
# controller (e.g. `kill -9` the pid in mango-bars.pid, not its process
# group) leaves this orphan holding mango-bars.lock — a fresh bars.sh's
# `flock -n 9` then refuses to start, forever, with no message. Measured live.
done 3< <(exec 9>&-
	mmsg watch all-monitors 2> /dev/null |
	jq -r --unbuffered '[.monitors[].name] | sort | join(" ")' 2> /dev/null)

wait
