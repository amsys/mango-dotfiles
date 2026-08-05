# Shared event-stream -> waybar-signal loop, for the continuous modules that
# exist only to refresh other modules the moment something changes.
#
# Sourced, never executed. bash, not POSIX: `coproc` is the whole point.
#
#   . "$(dirname "$0")/watch.sh"
#   watch_loop "mmsg watch all-tags"        # signals $SIGNAL, debounced
#
# Optionally define `watch_match <line>` before calling to filter the stream;
# lines it rejects do not schedule a refresh.
#
# Why coproc rather than the obvious `cmd | while read`: in a pipeline the
# stream producer is a sibling process, so when waybar terminates the module it
# kills the script and leaves `mmsg watch` / `pactl subscribe` running forever.
# Every waybar restart then leaks another one. coproc hands back a pid a trap
# can kill, which is exactly why net-watch.sh is built this way.

# A burst of events arrives per transition (10 lines for one window map); wait
# for it to go quiet, then signal once.
WATCH_DEBOUNCE=0.15
# Only bounds how long a dropped signal goes unnoticed — EOF ends the read
# immediately whatever this is, so it can be generous.
WATCH_IDLE=60

watch_loop() { # command line to run as the event source
	# `exec` matters: without it the brace group stays a subshell, $EV_PID names
	# the subshell, and killing it orphans the real stream anyway.
	coproc EV { exec $1 2> /dev/null; }
	# bash unsets EV_PID the moment the coproc reaps, so guard the expansion —
	# under `set -u` a bare one turns the exit trap into an error.
	trap '[ -n "${EV_PID:-}" ] && kill "$EV_PID" 2> /dev/null; exit 0' EXIT INT TERM

	# Refresh once at startup: waybar may have run the modules before the
	# compositor or the sound server was ready to answer.
	local pend=1 tmo rc line
	while :; do
		[ "$pend" = 1 ] && tmo=$WATCH_DEBOUNCE || tmo=$WATCH_IDLE
		IFS= read -r -u "${EV[0]}" -t "$tmo" line
		rc=$?

		if [ "$rc" = 0 ]; then
			# Not a `continue`-free branch by accident: the burst is still
			# arriving, so hold the refresh until it goes quiet.
			if ! command -v watch_match > /dev/null 2>&1 || watch_match "$line"; then
				pend=1
			fi
			continue
		fi

		# read(1) returns >128 on timeout and 1 on EOF. EOF means the source
		# went away — exit and let waybar's restart-interval bring us back.
		[ "$rc" -le 128 ] && exit 0

		if [ "$pend" = 1 ]; then
			pkill -RTMIN+"$SIGNAL" waybar
			pend=0
		fi
	done
}
