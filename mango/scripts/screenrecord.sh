#!/usr/bin/env bash
# Start/stop a screen recording. wf-recorder has no toggle of its own — it runs
# in the foreground until signalled — so a keybind needs this wrapper on both
# ends: one press starts it, the next stops it.
#
# Usage:
#   screenrecord.sh            # toggle, region selected with slurp
#   screenrecord.sh --screen   # toggle, whole output, no selection step
#   screenrecord.sh test       # self-check
set -u

OUTDIR="${XDG_VIDEOS_DIR:-$HOME/Videos}/Recordings"
# Where the in-progress filename lives, so the stopping press can name the file
# it just saved. Runtime dir, not /tmp: it is per-user and cleared on logout, so
# a crashed recorder cannot leave a stale path behind for the next session.
STATE="${XDG_RUNTIME_DIR:-/tmp}/screenrecord.path"

notify() { notify-send -a screenrecord -u low "$@"; }

# -x so this never matches the wrapper itself or an editor holding the script
# open; wf-recorder is the only thing that should ever match.
is_recording() { pkill -0 -x wf-recorder 2>/dev/null; }

if [ "${1:-}" = "test" ]; then
	# The two things that actually break silently: a filename template that
	# produces a colon (unplayable on anything mounted FAT/exFAT), and a state
	# file that outlives the recorder.
	name="Recording_$(date '+%Y-%m-%d_%H.%M.%S').mp4"
	case "$name" in
	*:*) echo "fail: timestamp contains a colon"; exit 1 ;;
	esac
	printf '%s\n' "/nonexistent/x.mp4" >"$STATE.test"
	[ "$(cat "$STATE.test")" = "/nonexistent/x.mp4" ] || { echo "fail: state roundtrip"; exit 1; }
	rm -f "$STATE.test"
	command -v wf-recorder >/dev/null || { echo "fail: wf-recorder not installed"; exit 1; }
	command -v slurp >/dev/null || { echo "fail: slurp not installed"; exit 1; }
	echo "ok"
	exit 0
fi

if is_recording; then
	# SIGINT, not SIGTERM or SIGKILL: wf-recorder traps INT to flush the muxer
	# and write the moov atom. Killed any other way the .mp4 exists but will
	# not play.
	pkill -INT -x wf-recorder
	saved=$(cat "$STATE" 2>/dev/null)
	rm -f "$STATE"
	# Flushing the moov atom takes a moment after the signal; wait for
	# wf-recorder to actually exit (bounded, so a wedged encoder can't hang
	# the keybind forever) before trusting the file on disk.
	n=0
	while is_recording && [ "$n" -lt 30 ]; do
		sleep 0.1
		n=$((n + 1))
	done
	if [ -n "$saved" ] && [ -s "$saved" ]; then
		notify "Recording saved" "$(basename "$saved")"
	else
		notify "Recording failed" "No file was written."
	fi
	exit 0
fi

mkdir -p "$OUTDIR"
file="$OUTDIR/Recording_$(date '+%Y-%m-%d_%H.%M.%S').mp4"

if [ "${1:-}" = "--screen" ]; then
	geom=""
else
	# slurp writes nothing and exits non-zero when cancelled with Escape. Bail
	# quietly rather than recording the whole screen by accident.
	geom=$(slurp) || exit 0
	[ -n "$geom" ] || exit 0
fi

# Backgrounded and disowned: mango's spawn runs this script and waits, so a
# foreground wf-recorder would block the compositor's spawn slot for the whole
# recording. Spawned and checked BEFORE the state file/toast are written: a
# bad slurp geometry or a full disk otherwise recorded a meeting that never
# existed, with a confirming "Recording started" toast to match.
if [ -n "$geom" ]; then
	wf-recorder -g "$geom" -f "$file" >/dev/null 2>&1 &
else
	wf-recorder -f "$file" >/dev/null 2>&1 &
fi
pid=$!
disown

sleep 0.5
if ! kill -0 "$pid" 2>/dev/null; then
	notify "Recording failed" "wf-recorder exited immediately."
	exit 1
fi

printf '%s\n' "$file" >"$STATE"
notify "Recording started" "Press the same key again to stop"
