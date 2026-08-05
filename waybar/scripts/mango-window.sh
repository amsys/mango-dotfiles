#!/usr/bin/env bash
# Active-window module for waybar: dim app id over the window title,
# falling back to "Desktop" when nothing is focused (design ported from end-4/dots-hyprland).
# Prints the current state once, then follows mango's focus-change stream.
#
#   mango-window.sh        custom/window exec — emit JSON, then follow the stream
#   mango-window.sh test    assert SIGTERM takes the whole pipeline with it
set -u

JQ_FILTER='
def esc: gsub("&"; "&amp;") | gsub("<"; "&lt;") | gsub(">"; "&gt;");
(.appid // "Desktop") as $appid
| (.title // "") as $title
| ($title | if length > 45 then .[0:45] + "…" else . end) as $shown
| {
    text: (
      "<span size=\"small\" alpha=\"70%\">" + ($appid | esc) + "</span>"
      + (if $shown == "" then "" else "\n" + ($shown | esc) end)
    ),
    tooltip: (if $title == "" then $appid else $title end)
  }
'

# ---------------------------------------------------------------- selftest

if [ "${1:-}" = "test" ]; then
	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT
	# A stub stream source, so the check needs neither a compositor nor a real
	# focus event: `mmsg get` must still terminate, `mmsg watch` must block.
	printf '#!/bin/sh\n[ "$1" = watch ] && exec sleep 300\nexit 0\n' > "$T/mmsg"
	chmod +x "$T/mmsg"

	# setsid, not a bare `&`: the whole point is that `kill 0` reaches the
	# module's own process group and nothing else, which is only meaningful if
	# the leader has a group of its own — exactly how waybar spawns it.
	PATH="$T:$PATH" setsid bash "$0" > /dev/null 2>&1 &
	LEADER=$!
	# The pipeline forks three processes; wait for them rather than racing.
	for _ in 1 2 3 4 5 6 7 8 9 10; do
		PG=$(ps -o pgid= -p "$LEADER" 2> /dev/null | tr -d ' ')
		[ -n "$PG" ] && [ "$(pgrep -g "$PG" | wc -l)" -ge 3 ] && break
		sleep 0.2
	done
	[ -n "${PG:-}" ] || { echo "leader never started"; exit 1; }

	kill -TERM "$LEADER" 2> /dev/null
	for _ in 1 2 3 4 5 6 7 8 9 10; do
		LEFT=$(pgrep -g "$PG" | wc -l)
		[ "$LEFT" = 0 ] && break
		sleep 0.2
	done

	# Survivors first, and only then `wait`: a leader that defers the trap is
	# still alive here, and waiting on it is the one thing guaranteed never to
	# return.
	if [ "$LEFT" != 0 ]; then
		echo "SIGTERM left $LEFT process(es) in group $PG:"
		ps -o pid,ppid,args -g "$PG" 2> /dev/null
		# KILL, not TERM: one way to fail this check is a leader that defers the
		# trap forever, and TERM is precisely what it is not answering.
		kill -KILL -"$PG" 2> /dev/null
		exit 1
	fi

	# An empty group is not on its own proof of a clean teardown: a trap that
	# re-enters itself also empties the group, by way of a stack overflow. 143 is
	# SIGTERM, which is how `kill 0` is meant to end this shell; 139 is the
	# SIGSEGV that says the handler ate itself.
	wait "$LEADER" 2> /dev/null
	RC=$?
	[ "$RC" = 143 ] || { echo "leader exited $RC, expected 143 (SIGTERM)"; exit 1; }

	echo "mango-window: all checks passed"
	exit 0
fi

# ---------------------------------------------------------------- module

# waybar SIGTERMs only the process it spawned — this script — and everything the
# pipeline forks (the `{ … }` subshell, jq, `mmsg watch`) is a grandchild it never
# touches. Once mango goes away the stream falls silent, so mmsg never writes and
# never takes the SIGPIPE that would end it: one triple leaked per session,
# forever. waybar puts each module in its own process group, so `kill 0` reaches
# exactly this module's tree. Run it by hand only from a shell with job control —
# under `sh -c ./mango-window.sh` this script shares the caller's group.
#
# `kill 0`, not `kill "$!"`: $! is the pipeline's last element (jq), and the
# subshell and mmsg would survive it.
#
# Every trap is cleared first, not just EXIT: `kill 0` signals this shell too, so
# leaving TERM installed re-enters the handler until bash dies of a stack
# overflow — a SIGSEGV where a clean exit was the whole point.
trap 'trap - EXIT INT TERM; kill 0' EXIT INT TERM

# Backgrounded with an explicit `wait`, not run in the foreground: bash defers a
# trap until the foreground job returns, and that job is the thing the trap
# exists to kill.
{ mmsg get focusing-client 2>/dev/null; mmsg watch focusing-client 2>/dev/null; } \
  | jq --unbuffered -c "$JQ_FILTER" &
wait
