#!/usr/bin/env bash
# Crash-resilient launch wrapper for mango.
#
# SDDM's wayland-session exec()s this, so it is the session's foreground
# process and mango stays a plain child in session-N.scope — seat, logind and
# DRM handling are byte-identical to launching mango directly.
#
# Clean logout (SUPER+m -> wlogout -> mmsg dispatch quit) exits 0 and ends the
# session. A crash — SIGABRT (134) from the wlroots lost-renderer assert after
# an i915 GT reset, SIGSEGV, ... — restarts mango in place instead of dropping
# to the login screen. See plans/iterative-watching-spring.md for the crash
# this addresses.
set -u

CGROUP=/sys/fs/cgroup$(cut -d: -f3 </proc/self/cgroup)

# mango setsid()s every exec-once child before exec, so each one is its own
# session leader — killing our process group misses all of them (verified:
# `nm -D /usr/bin/mango` has setsid@GLIBC_2.2.5, and the leaked pollers have
# pid == pgid == sid). After the 2026-08-12 GPU hang, powerkey.py /
# battery-guard.sh / ac-watch.sh were still running from the previous
# session — one extra copy per generation. The cgroup is the one thing they
# cannot setsid() out of. Shells started from kitty live in their own
# kitty-<pid>-0.scope under user@1000.service, outside this cgroup, so tmux
# servers and background jobs are not touched.
reap() {
	local pids pid
	[[ -r $CGROUP/cgroup.procs ]] || return 0
	mapfile -t pids <"$CGROUP/cgroup.procs"
	for pid in "${pids[@]}"; do
		[[ $pid == "$$" || $pid == "$PPID" ]] && continue # us, sddm-helper
		kill "-$1" "$pid" 2>/dev/null
	done
}

# loginctl terminate-session/-user SIGTERMs us too: end the session, do not
# race it by respawning.
trap 'exit 0' HUP INT TERM

fails=0
while :; do
	started=$SECONDS
	mango "$@"
	rc=$?
	reap TERM
	sleep 2 # also lets the GPU settle before the next DRM master grab
	reap KILL
	((rc == 0 || rc == 143)) && exit 0
	if ((SECONDS - started < 60)); then
		((++fails))
	else
		fails=0
	fi
	printf 'mango exited %d, restart %d/3\n' "$rc" "$fails" >&2
	((fails >= 3)) && exit "$rc" # broken config or driver: fall through to SDDM
done
