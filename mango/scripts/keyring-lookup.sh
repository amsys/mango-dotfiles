#!/usr/bin/env bash
# Fetch a secret from the freedesktop Secret Service (KeePassXC owns it here).
# Prints the secret on success; 'not found' (exit 1) or 'locked' (exit 2) so
# callers can tell "unlock your vault" from "add the entry" — see rofi/ai.sh.
#
# Usage: keyring-lookup.sh [application-attribute]   (default: mango)

APP="${1:-mango}"

# Resolve the default collection rather than assuming gnome-keyring's 'login' —
# KeePassXC names its collection after the database file, so the name differs
# per machine and cannot be hardcoded.
is_unlocked() {
	local path locked
	path=$(busctl --user call org.freedesktop.secrets /org/freedesktop/secrets \
		org.freedesktop.Secret.Service ReadAlias s default 2>/dev/null |
		awk -F'"' '{print $2}') || return 1
	[ -n "$path" ] || return 1
	locked=$(busctl --user get-property org.freedesktop.secrets "$path" \
		org.freedesktop.Secret.Collection Locked 2>/dev/null |
		awk '{print $2}') || return 1
	[ "$locked" = "false" ]
}

if [ "${1:-}" = "test" ]; then
	echo 'o "/org/freedesktop/secrets/collection/Default"' | awk -F'"' '{print $2}' |
		grep -qx '/org/freedesktop/secrets/collection/Default'
	echo 'b false' | awk '{print $2}' | grep -qx 'false'
	echo "ok"
	exit 0
fi

# The lock state is checked *before* secret-tool is called, not after: a lookup
# against a locked collection blocks on an unlock prompt, and a caller with no
# terminal to answer it waits forever (rofi/ai.sh runs its fetch detached, so
# that is a hung request and a stuck panel rather than a visible dialog).
if ! is_unlocked; then
	echo 'locked'
	exit 2
fi

# The vault can still be locked between the check above and this line. The cap
# turns that race into a slow 'not found' instead of an unbounded wait.
data=$(timeout 10 secret-tool lookup 'application' "$APP")
if [ -z "$data" ]; then
	echo 'not found'
	exit 1
fi
echo "$data"
