#!/usr/bin/env bash
# Rotates the password on valkey's 'user default' ACL line and updates the
# one client that reads it from a dotfile. Follows the system/rapl/ and
# system/vault/ precedent: sudo-gated, nothing under system/ is symlinked
# into ~/.config, so this script is the only way the rotation lands.
#
#   sudo ~/src/mango-dotfiles/system/valkey/rotate.sh
#   ~/src/mango-dotfiles/system/valkey/rotate.sh --self-test   # no root
#
# VAULT.md section 1.D flags valkey as reachable with no auth path review —
# this script is the fix for "the password never changes". The secret never
# reaches argv, stdout, stderr or a log: it moves only through bash
# variables, the REDISCLI_AUTH environment variable (valkey-cli's own
# documented input for this), and file contents written by redirection.
# The 'user default' line rewrite and the REDIS_URL line rewrite are each a
# single bash function, so --self-test can run both against a sample file
# with no root and no live valkey.
#
# check: rotate.sh --self-test
set -eu

CONF="/etc/valkey/valkey.conf"

# --- line rewrites (pure: read $1, write $2, set OLD_SECRET) --------------
#
# Both use only bash builtins (case, parameter expansion, read, printf) —
# no external command ever sees the secret as an argument.

rewrite_conf_line() {
	# Replaces the ACL password token (the field starting with ">") on the
	# 'user default' line. noglob is on for the split, so a '~*' or '+@all'
	# field is never taken as a pathname pattern.
	local src="$1" dst="$2" line field newline
	OLD_SECRET=""
	: >"$dst"
	set -f
	while IFS= read -r line || [ -n "$line" ]; do
		case "$line" in
		"user default "*)
			newline=""
			for field in $line; do
				case "$field" in
				">"*)
					OLD_SECRET="${field#>}"
					field=">$NEW_SECRET"
					;;
				esac
				newline="$newline$field "
			done
			printf '%s\n' "${newline% }" >>"$dst"
			;;
		*)
			printf '%s\n' "$line" >>"$dst"
			;;
		esac
	done <"$src"
	set +f
}

rewrite_fish_line() {
	# Replaces the password between 'redis://:' and '@' on the REDIS_URL line.
	local src="$1" dst="$2" line prefix rest after
	OLD_SECRET=""
	: >"$dst"
	while IFS= read -r line || [ -n "$line" ]; do
		case "$line" in
		*'redis://:'*'@'*)
			prefix="${line%%redis://:*}"
			rest="${line#*redis://:}"
			OLD_SECRET="${rest%%@*}"
			after="${rest#*@}"
			printf '%s\n' "${prefix}redis://:${NEW_SECRET}@${after}" >>"$dst"
			;;
		*)
			printf '%s\n' "$line" >>"$dst"
			;;
		esac
	done <"$src"
}

self_test() {
	local tmpdir rc=0
	tmpdir="$(mktemp -d)"
	trap 'rm -rf "$tmpdir"' RETURN

	NEW_SECRET="NEWTESTSECRET123"

	printf 'bind * -::*\nuser default on ~* +@all >oldsecretvalue\nmaxmemory 100mb\n' \
		>"$tmpdir/valkey.conf"
	rewrite_conf_line "$tmpdir/valkey.conf" "$tmpdir/valkey.conf.out"
	[ "$OLD_SECRET" = "oldsecretvalue" ] || rc=1
	grep -Fqx 'user default on ~* +@all >NEWTESTSECRET123' "$tmpdir/valkey.conf.out" || rc=1

	printf 'set -gx PATH x\nset -x REDIS_URL "redis://:oldpw123@10.0.0.1:6379/0"\nset -x FOO bar\n' \
		>"$tmpdir/claude.fish"
	rewrite_fish_line "$tmpdir/claude.fish" "$tmpdir/claude.fish.out"
	[ "$OLD_SECRET" = "oldpw123" ] || rc=1
	grep -Fqx 'set -x REDIS_URL "redis://:NEWTESTSECRET123@10.0.0.1:6379/0"' \
		"$tmpdir/claude.fish.out" || rc=1

	if [ "$rc" -eq 0 ]; then
		echo PASS
	else
		echo FAIL
	fi
	return "$rc"
}

[ "${1:-}" = "--self-test" ] && { self_test; exit $?; }

# --- real run: needs root ---------------------------------------------------

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER to know whose REDIS_URL to update" >&2; exit 1; }
TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
[ -n "$TARGET_HOME" ] && [ -d "$TARGET_HOME" ] || { echo "no home directory for $TARGET_USER" >&2; exit 1; }

command -v openssl >/dev/null 2>&1 || { echo "missing dependency: openssl" >&2; exit 1; }
command -v valkey-cli >/dev/null 2>&1 || { echo "missing dependency: valkey-cli" >&2; exit 1; }
[ -f "$CONF" ] || { echo "$CONF not found" >&2; exit 1; }

FISH_CONF="$TARGET_HOME/.config/fish/conf.d/claude.fish"

echo "==> generating a new secret"
NEW_SECRET="$(openssl rand -hex 32)"

echo "==> rewriting $CONF"
TMP_CONF="$(mktemp "${CONF}.XXXXXX")"
rewrite_conf_line "$CONF" "$TMP_CONF"
if [ -z "$OLD_SECRET" ]; then
	rm -f "$TMP_CONF"
	echo "no 'user default' password field found in $CONF — nothing changed" >&2
	exit 1
fi
OLD_VALKEY_SECRET="$OLD_SECRET"
install -m 0640 -o root -g valkey "$TMP_CONF" "$CONF"
rm -f "$TMP_CONF"

echo "==> restarting valkey"
systemctl restart valkey

if [ -f "$FISH_CONF" ]; then
	echo "==> rewriting $FISH_CONF"
	TMP_FISH="$(mktemp "${FISH_CONF}.XXXXXX")"
	rewrite_fish_line "$FISH_CONF" "$TMP_FISH"
	install -m 0600 -o "$TARGET_USER" -g "$TARGET_USER" "$TMP_FISH" "$FISH_CONF"
	rm -f "$TMP_FISH"
else
	echo "    $FISH_CONF not found — skipping"
fi

echo "==> verifying PING with the new secret"
if ! REDISCLI_AUTH="$NEW_SECRET" valkey-cli -h 127.0.0.1 ping | grep -qx PONG; then
	echo "    PING failed. valkey.conf and REDIS_URL both now hold the new" >&2
	echo "    secret; check 'systemctl status valkey' and 'journalctl -u valkey'." >&2
	exit 1
fi
echo "    PONG"

echo "==> checking $TARGET_HOME/.config and $TARGET_HOME/work for the old secret"
for d in "$TARGET_HOME/.config" "$TARGET_HOME/work"; do
	[ -d "$d" ] || continue
	grep -rlF \
		--exclude-dir=.git --exclude-dir=node_modules --exclude-dir=vendor \
		-f <(printf '%s\n' "$OLD_VALKEY_SECRET") "$d" 2>/dev/null || true
done

cat <<EOF

Done. Any path listed above still holds the old secret and needs a manual
update.

Rollback: none — the old secret no longer works on the running valkey.
Recovery needs a backup of $CONF and $FISH_CONF taken before this run.
EOF
