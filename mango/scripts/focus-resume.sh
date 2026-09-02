#!/usr/bin/env bash
# Ready-to-Resume prompt — FOCUS.md §2.1: one line, where you are + the exact
# next action, written the moment a named work block ends. Externalizes the
# open loop so attention residue drops, instead of carrying it silently into
# the break.
#
#   focus-resume.sh "<task>"   spawned by mango-bard's pomo.rs (Stage B
#                               "alert") at the work bell, only when a task
#                               was named — never launched directly.
#   focus-resume.sh test       assert the lock guard
set -euo pipefail

if [ "${1:-}" = "test" ]; then
	TMP=$(mktemp -d)
	trap 'rm -rf "$TMP"' EXIT
	export PATH="$TMP:$PATH"

	printf '#!/usr/bin/env bash\ntouch "$MARK"\n' >"$TMP/rofi"
	chmod +x "$TMP/rofi"

	# swaylock running -> the guard must skip rofi entirely.
	printf '#!/usr/bin/env bash\nexit 0\n' >"$TMP/pidof"
	chmod +x "$TMP/pidof"
	MARK="$TMP/marked-locked" "$0" "task"
	[ -e "$TMP/marked-locked" ] && { echo "guard did not skip rofi while locked"; exit 1; }

	# swaylock not running -> rofi must still run (its empty answer exits
	# the script before mango-bard/DATA_DIR are ever touched).
	printf '#!/usr/bin/env bash\nexit 1\n' >"$TMP/pidof"
	chmod +x "$TMP/pidof"
	MARK="$TMP/marked-unlocked" "$0" "task"
	[ -e "$TMP/marked-unlocked" ] || { echo "guard skipped rofi while unlocked"; exit 1; }

	echo "ok"
	exit 0
fi

TASK="${1:?usage: focus-resume.sh <task>}"

# Locked: same reasoning as focus-break.sh's own guard — no rofi window
# behind the lock surface. No `timeout` here (unlike focus-break.sh):
# this prompt captures typed text, and a timeout would silently discard it.
pidof swaylock >/dev/null && exit 0

LINE=$(rofi -dmenu -p "Resume \"$TASK\" — next:" -theme ~/.config/rofi/focus-input.rasi -l 0)
[ -n "$LINE" ] || exit 0

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/mango-focus"
mkdir -p "$DATA_DIR"
printf '%s\t%s\t%s\n' "$(printf '%(%Y-%m-%dT%H:%M:%S)T' -1)" "$TASK" "$LINE" \
	>> "$DATA_DIR/resume.md"
mango-bard pomo resume "$TASK — $LINE" -q
