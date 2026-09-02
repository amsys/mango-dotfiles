#!/usr/bin/env bash
# Full-screen phase-boundary overlay — FOCUS.md §4.3: a blocking, screen-
# filling banner instead of a corner toast you learn to ignore. Water
# reminder rides the break bell itself (§1: sip at every break), so there is
# no separate water timer.
#
#   focus-break.sh "<phase label>" <minutes>   spawned by mango-bard's
#                                               pomo.rs at every phase
#                                               boundary — never launched
#                                               directly.
#   focus-break.sh test                        assert the lock guard and
#                                               the overlay timeout
set -euo pipefail

# ponytail: fixed ceiling, not phase-aware — raise it only if a banner ever
# needs to outlive 2 minutes. `: "${VAR:=...}"` (not a plain assignment) so
# the self-test below can override it per invocation.
: "${OVERLAY_TIMEOUT:=120}"

if [ "${1:-}" = "test" ]; then
	TMP=$(mktemp -d)
	trap 'rm -rf "$TMP"' EXIT
	export PATH="$TMP:$PATH"

	printf '#!/usr/bin/env bash\ntouch "$MARK"\n' >"$TMP/rofi"
	chmod +x "$TMP/rofi"

	# swaylock running -> the guard must skip rofi entirely.
	printf '#!/usr/bin/env bash\nexit 0\n' >"$TMP/pidof"
	chmod +x "$TMP/pidof"
	MARK="$TMP/marked-locked" "$0" "Break" 5
	[ -e "$TMP/marked-locked" ] && { echo "guard did not skip rofi while locked"; exit 1; }

	# swaylock not running -> the overlay must still fire.
	printf '#!/usr/bin/env bash\nexit 1\n' >"$TMP/pidof"
	chmod +x "$TMP/pidof"
	MARK="$TMP/marked-unlocked" "$0" "Break" 5
	[ -e "$TMP/marked-unlocked" ] || { echo "guard skipped rofi while unlocked"; exit 1; }

	# a deaf rofi must not hang the script past the timeout.
	printf '#!/usr/bin/env bash\nsleep 999\n' >"$TMP/rofi"
	chmod +x "$TMP/rofi"
	START=$(date +%s)
	OVERLAY_TIMEOUT=1 "$0" "Break" 5 || true
	ELAPSED=$(( $(date +%s) - START ))
	[ "$ELAPSED" -lt 5 ] || { echo "timeout did not bound the overlay: ${ELAPSED}s"; exit 1; }

	echo "ok"
	exit 0
fi

LABEL="${1:?usage: focus-break.sh <phase-label> <minutes>}"
MINUTES="${2:?usage: focus-break.sh <phase-label> <minutes>}"

# Locked: no window should ever be created behind the lock surface — a rofi
# layer surface mapped while ext-session-lock-v1 is held can't get exclusive
# keyboard, and mango does not re-scan it on unlock, leaving it visible and
# deaf (the `killall rofi` bug this guard exists to prevent). Every lock
# path is meant to pause the pomodoro instead (sleep-lock.py's `lock`
# subcommand), so this is defense in depth, not the primary fix.
pidof swaylock >/dev/null && exit 0

MSG="$LABEL — $MINUTES min"
case "$LABEL" in
*[Bb]reak*) MSG="$MSG — sip water" ;;
esac

# `-e` (message mode): no candidates, no filtering, just the text — Esc or
# Enter dismisses. focus.rasi sizes the window full-screen. `timeout` is the
# backstop for any overlay that still ends up deaf despite the guard above.
timeout "$OVERLAY_TIMEOUT" rofi -e "$MSG" -theme ~/.config/rofi/focus.rasi
