#!/usr/bin/env bash
# Store one clipboard entry in cliphist, but never store a password.
#
# wl-paste sets CLIPBOARD_STATE=sensitive for the spawned command when the
# clipboard offer carries the MIME type x-kde-passwordManagerHint. KeePassXC
# sets that type on every copy. cliphist has no filter of its own, so without
# this guard each copied password goes to ~/.cache/cliphist/db in plain text
# and stays there — the db holds 750 entries and Alt+v shows them all.
#
# KeePassXC clears the system clipboard 10 seconds after a copy. That timeout
# does not help: cliphist has already written the value to disk.
#
# This reads the variable instead of asking wl-paste for the MIME list again.
# wl-paste examined the live offer, so there is no second query and no race
# with the next copy.
set -uo pipefail

# True when the entry may be stored. Split out so the selftest can check the
# decision without a live clipboard.
keep() { [ "${1:-}" != sensitive ]; }

if [ "${1:-}" = test ]; then
	keep sensitive && { echo "a password would be stored"; exit 1; }
	keep data || { echo "normal text was dropped"; exit 1; }
	keep nil || { echo "an empty clipboard was dropped"; exit 1; }
	keep || { echo "an unset state was dropped"; exit 1; }
	keep '' || { echo "an empty state was dropped"; exit 1; }
	echo "ok"
	exit 0
fi

if ! keep "${CLIPBOARD_STATE:-}"; then
	# Read the offer and discard it. An immediate exit leaves the writer on
	# the other end of stdin with a closed pipe.
	cat > /dev/null
	exit 0
fi

exec cliphist store
