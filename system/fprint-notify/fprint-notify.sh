#!/usr/bin/env bash
# Shows a persistent mako notification while a fingerprint is requested.
#
# Modes, selected by argument and PAM_TYPE:
#   PAM auth phase    — show the notification; a detached worker dismisses
#                       it when the fingerprint request ends.
#   PAM account phase — auth succeeded, the request is over; dismiss.
#   click             — mako left-click binding; focus the window whose
#                       sudo asked for the finger. Does not dismiss.
#   _worker           — internal; the detached show-and-wait process.
#
# PAM setup in /etc/pam.d/sudo (both pam_exec lines):
#
#   auth    optional   pam_exec.so quiet /usr/local/bin/mango-fprint-notify
#   auth    sufficient pam_fprintd.so
#   account optional   pam_exec.so quiet /usr/local/bin/mango-fprint-notify
#   account include    system-auth
#
# pam_exec exports PAM_SERVICE, PAM_USER, PAM_TYPE, PAM_TTY and PAM_RUSER
# (man 8 pam_exec). PAM_SERVICE is the only source of the asker's identity.
#
# There is deliberately no `set -e` here. This script runs INSIDE the auth
# path of sudo. It must always exit 0 and it must always exit fast. A
# notification is never worth a failed or delayed sudo.

# The process that called PAM — sudo itself. $PPID only points at it in
# the direct pam_exec invocation; the runuser re-exec and the detached
# worker get it handed down through CALLER_PID instead.
caller_pid="${CALLER_PID:-$PPID}"

# Absolute path to this script: $0 is relative when invoked as
# `bash fprint-notify.sh`, and the re-exec paths need an absolute one.
SELF="$(readlink -f "${BASH_SOURCE[0]}")"

RUN_DIR="/run/user/$(id -u)"
STATE="$RUN_DIR/mango-fprint-notify.state"
# 48px copy of the breeze-dark fingerprint glyph — mako caps icons at
# max-icon-size=48 but does not upscale, so the 24px system icon renders
# small. install.sh puts this in place.
# ponytail: light-on-dark only; add a matugen-themed copy if light mode
# ever lands.
ICON=/usr/local/share/pixmaps/mango-fprint-notify.svg
# pam_fprintd stops asking after 30s (its default timeout). The worker
# holds the notification a little longer than that, never much longer.
CAP=35

# mmsg needs the compositor socket. The PAM environment is empty, so probe
# the sockets in the runtime dir, newest first (same idea as
# rescue-outputs.sh resolve_socket).
find_socket() {
	local c
	[ -n "${MANGO_INSTANCE_SIGNATURE:-}" ] && return 0
	# mango-[0-9]*: the compositor socket is mango-<pid>.sock. The glob
	# must not catch mango-bard.sock, which answers `get version` too
	# but returns "unknown" for every real query.
	# find + sort, not `ls -t`: parsing ls is fragile. Process substitution
	# keeps this loop in the current shell, so the export below survives —
	# a `| while read` pipeline would run it in a subshell and lose it.
	while IFS= read -r c; do
		if MANGO_INSTANCE_SIGNATURE=$c timeout 2 mmsg get version > /dev/null 2>&1; then
			export MANGO_INSTANCE_SIGNATURE=$c
			return 0
		fi
	done < <(find "$RUN_DIR" -maxdepth 1 -name 'mango-[0-9]*.sock' \
		-printf '%T@ %p\n' 2> /dev/null | sort -rn | cut -d' ' -f2-)
	return 1
}

# --- click mode (run by mako as the session user) -----------------------
if [ "${1:-}" = "click" ]; then
	read -r _nid cid 2> /dev/null < "$STATE" || exit 0
	[ -n "$cid" ] || exit 0
	find_socket || exit 0
	# focusid views the client's tag, restores it and focuses it — the
	# same one-dispatch jump rofi/window.sh uses.
	exec timeout 2 mmsg dispatch focusid client,"$cid" > /dev/null 2>&1
fi

# --- PAM gates ----------------------------------------------------------

# No service name means this was not called from PAM. Do nothing.
service="${PAM_SERVICE:-}"
[ -n "$service" ] || exit 0

# PAM_RUSER is the requesting user — the account that owns the graphical
# session we want to reach. PAM_USER is the account being authenticated;
# the same name for a default sudo, but not for `rootpw`-style setups.
user="${PAM_RUSER:-}"
[ -n "$user" ] || user="${PAM_USER:-}"
[ -n "$user" ] || exit 0

uid="$(id -u "$user" 2> /dev/null)" || exit 0
[ -n "$uid" ] || exit 0

# No user bus means no graphical session — an SSH sudo or a TTY login.
bus="/run/user/$uid/bus"
[ -S "$bus" ] || exit 0

# Under sudo, pam_exec starts this script with real uid = the invoking
# user and effective uid = 0. Bash then drops the effective uid to the
# real uid (its setuid protection), so during a sudo auth this script IS
# the invoking user already. Only a real-root caller (the install
# self-test) takes the runuser branch. runuser uses PAM service
# "runuser", not "sudo", so it cannot recurse back into this script.
me="$(id -u)"
if [ "$me" -eq 0 ]; then
	setsid runuser -u "$user" -- env \
		"PAM_SERVICE=$service" "PAM_TYPE=${PAM_TYPE:-auth}" \
		"PAM_TTY=${PAM_TTY:-}" "PAM_RUSER=$user" \
		"CALLER_PID=$caller_pid" \
		"$SELF" > /dev/null 2>&1 &
	exit 0
elif [ "$me" -ne "$uid" ]; then
	exit 0
fi

export DBUS_SESSION_BUS_ADDRESS="unix:path=$bus"

# --- worker mode (detached; never blocks sudo) --------------------------
if [ "${1:-}" = "_worker" ]; then
	tty="${PAM_TTY#/dev/}"
	body="$service on ${tty:-?}"

	# Find the window the request came from: climb the caller's process
	# tree (sudo → shell → terminal) until a pid owns a mango client.
	cid=""
	if find_socket; then
		clients="$(timeout 2 mmsg get all-clients 2> /dev/null)"
		pid=$caller_pid
		for _ in {1..15}; do
			[ "${pid:-0}" -gt 1 ] || break
			m="$(jq -r --argjson p "$pid" \
				'first(.clients[] | select(.pid == $p)) | "\(.id) \(.tags[0] // "") \(.monitor)"' \
				<<< "$clients" 2> /dev/null)"
			if [ -n "$m" ]; then
				read -r cid tag mon <<< "$m"
				[ -n "$tag" ] && body="$body · tag $tag · $mon"
				break
			fi
			# ppid is the 2nd field after the ')' that ends comm.
			pid="$(sed 's/^.*) //' "/proc/$pid/stat" 2> /dev/null | cut -d' ' -f2)"
		done
	fi

	# The mako rule [app-name=fprint] makes this persist (default-timeout=0);
	# the synchronous hint makes a retry replace it instead of stacking.
	nid="$(timeout 3 notify-send -p -a fprint -u normal -i "$ICON" \
		-h string:x-canonical-private-synchronous:fprint \
		"Fingerprint requested" "$body" 2> /dev/null)"
	[ -n "$nid" ] || exit 0
	printf '%s %s\n' "$nid" "$cid" > "$STATE"

	# Hold until the request ends: the account phase removes the state
	# file on success, the caller dying covers Ctrl-C and failure, and
	# CAP covers pam_fprintd giving up while sudo waits for a password.
	end=$((SECONDS + CAP))
	while [ "$SECONDS" -lt "$end" ]; do
		read -r cur _ 2> /dev/null < "$STATE" || exit 0
		[ "$cur" = "$nid" ] || exit 0 # a newer request took over
		[ -d "/proc/$caller_pid" ] || break
		sleep 0.5
	done
	timeout 3 makoctl dismiss -n "$nid" > /dev/null 2>&1
	rm -f "$STATE"
	exit 0
fi

# --- PAM entry points ---------------------------------------------------
case "${PAM_TYPE:-auth}" in
auth)
	# Detached: a wedged notification daemon or compositor must not
	# hold up the fingerprint prompt. Everything slow lives in _worker.
	CALLER_PID="$caller_pid" setsid "$SELF" _worker > /dev/null 2>&1 &
	;;
account)
	# Runs after a successful auth — the fingerprint request is over.
	# Also runs for cached-timestamp sudos; then the state file is
	# already gone and this is a no-op.
	if read -r nid _ 2> /dev/null < "$STATE" && [ -n "$nid" ]; then
		rm -f "$STATE"
		setsid timeout 3 makoctl dismiss -n "$nid" > /dev/null 2>&1 &
	fi
	;;
esac

exit 0
