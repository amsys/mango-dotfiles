#!/usr/bin/env bash
# Waits for KeePassXC's freedesktop Secret Service collection to unlock,
# then starts everything else that depends on it having secrets available.
set -euo pipefail

parse_path() { awk -F'"' '{print $2}'; }
parse_locked() { awk '{print $2}'; }

is_unlocked() {
	local path
	path=$(busctl --user call org.freedesktop.secrets /org/freedesktop/secrets \
		org.freedesktop.Secret.Service ReadAlias s default 2>/dev/null | parse_path) || return 1
	[ -n "$path" ] || return 1
	local locked
	locked=$(busctl --user get-property org.freedesktop.secrets "$path" \
		org.freedesktop.Secret.Collection Locked 2>/dev/null | parse_locked) || return 1
	[ "$locked" = "false" ]
}

if [ "${1:-}" = "test" ]; then
	echo 'o "/org/freedesktop/secrets/collection/Default"' | parse_path | grep -qx '/org/freedesktop/secrets/collection/Default'
	echo 'b false' | parse_locked | grep -qx 'false'
	echo 'b true' | parse_locked | grep -qx 'true'
	echo "ok"
	exit 0
fi

# ponytail: polls every 1s with no timeout, waits forever if the database is never unlocked
until is_unlocked; do
	sleep 1
done

# mango applies its `env=` block after forking exec-once children, so this script
# froze the pre-env environment at fork — set it here or Qt gets no platform theme.
export QT_QPA_PLATFORMTHEME=kde
nextcloud &
