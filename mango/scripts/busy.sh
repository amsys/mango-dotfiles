#!/usr/bin/env bash
# Is something in flight that a suspend or a shutdown would ruin?
#
# Two callers ask this at the two moments it matters: battery-guard.sh before
# it auto-suspends a dying laptop, and powermenu.sh before it offers you a
# Shutdown button. Hence a file of its own rather than the same ten lines
# copied into both.
#
#   busy.sh         exit 0 and print why, or exit 1 in silence
#   busy.sh test    assert both detectors against a temp tree
set -uo pipefail

# The pacman lock is the whole story for package operations: pacman, paru and
# yay all take it, it exists for exactly the length of the transaction, and it
# needs no process-list scraping. A half-applied transaction is the worst thing
# on this list to interrupt.
LCK="${MANGO_PACMAN_LCK:-/var/lib/pacman/db.lck}"

# Browsers write to a temp name until the transfer completes: Firefox .part,
# Chromium .crdownload, a few others .download. An abandoned one would block
# suspend forever, so only count files touched in the last few minutes.
DL="${MANGO_DOWNLOAD_DIR:-$HOME/Downloads}"
DL_FRESH_MIN="${MANGO_DOWNLOAD_FRESH_MIN:-5}"

# reason string, or empty when nothing is happening
busy_reason() {
	local reasons=() n
	[ -e "$LCK" ] && reasons+=("a package transaction is in progress")

	if [ -d "$DL" ]; then
		n=$(find "$DL" -maxdepth 1 -type f \
			\( -name '*.part' -o -name '*.crdownload' -o -name '*.download' \) \
			-mmin "-$DL_FRESH_MIN" 2>/dev/null | wc -l)
		[ "$n" -gt 0 ] && reasons+=("$n unfinished download$([ "$n" -gt 1 ] && echo s)")
	fi

	[ ${#reasons[@]} -eq 0 ] && return
	local IFS=$'\n'
	printf '%s' "${reasons[0]}"
	for ((i = 1; i < ${#reasons[@]}; i++)); do printf ' and %s' "${reasons[i]}"; done
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = test ]; then
	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT
	mkdir -p "$T/dl"
	LCK="$T/db.lck" DL="$T/dl"

	[ -z "$(busy_reason)" ] || { echo "clean tree should be idle: $(busy_reason)"; exit 1; }

	touch "$T/db.lck"
	[ "$(busy_reason)" = "a package transaction is in progress" ] || { echo "lock not seen: $(busy_reason)"; exit 1; }
	rm "$T/db.lck"

	touch "$T/dl/iso.part"
	[ "$(busy_reason)" = "1 unfinished download" ] || { echo "fresh .part not seen: $(busy_reason)"; exit 1; }
	touch "$T/dl/deb.crdownload"
	[ "$(busy_reason)" = "2 unfinished downloads" ] || { echo "plural wrong: $(busy_reason)"; exit 1; }

	# both at once, so the joined phrasing gets exercised too
	touch "$T/db.lck"
	[ "$(busy_reason)" = "a package transaction is in progress and 2 unfinished downloads" ] || { echo "join wrong: $(busy_reason)"; exit 1; }
	rm "$T/db.lck"

	# a download abandoned an hour ago must not pin the machine awake
	touch -d '1 hour ago' "$T/dl/iso.part" "$T/dl/deb.crdownload"
	[ -z "$(busy_reason)" ] || { echo "stale .part should be ignored: $(busy_reason)"; exit 1; }

	# a completed file next to them is not a download in progress
	touch "$T/dl/done.iso"
	[ -z "$(busy_reason)" ] || { echo "finished file counted: $(busy_reason)"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- main

REASON=$(busy_reason)
[ -n "$REASON" ] || exit 1
printf '%s\n' "$REASON"
