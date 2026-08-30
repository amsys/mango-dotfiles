#!/usr/bin/env bash
# Start KeePassXC at login. It shows its normal unlock prompt and you type the
# password by hand. This script does not unlock the vault any more: the old
# SDDM PAM stash (/run/keepassxc-unlock/$USER) is retired.
# keepassxc.ini does the window part: MinimizeOnStartup=false keeps the prompt
# visible, MinimizeAfterUnlock=true hides the window after you unlock it.
set -uo pipefail

parse_db() { sed -n 's/^[[:space:]]*env=MANGO_KEEPASS_DB,//p' | tail -1; }

if [ "${1:-}" = "test" ]; then
	printf 'env=MANGO_KEEPASS_DB,/x/y.kdbx\n' | parse_db | grep -qx '/x/y.kdbx'
	printf '\tenv=MANGO_KEEPASS_DB,/x/y.kdbx\n' | parse_db | grep -qx '/x/y.kdbx'
	# last one wins, and a commented line is not a setting
	printf 'env=MANGO_KEEPASS_DB,/a.kdbx\nenv=MANGO_KEEPASS_DB,/b.kdbx\n' |
		parse_db | grep -qx '/b.kdbx'
	[ -z "$(printf '#env=MANGO_KEEPASS_DB,/x.kdbx\n' | parse_db)" ]
	[ -z "$(printf 'env=OTHER,/x.kdbx\n' | parse_db)" ]
	echo "ok"
	exit 0
fi

# The database path is per-machine, so it is not in this repo. It comes from
# MANGO_KEEPASS_DB, falling back to the same variable set in mango/local.conf
# (untracked). The fallback exists because mango applies its env= block only
# *after* forking exec-once children, and this script is an exec-once — so the
# variable is reliably readable from the file but not from the environment.
# KeePassXC does not reopen a last-used database when given no path, so there
# is nothing sensible to default to: with neither set, this exits and you
# start KeePassXC by hand.
DB="${MANGO_KEEPASS_DB:-$(parse_db \
	<"${XDG_CONFIG_HOME:-$HOME/.config}/mango/local.conf" 2>/dev/null)}"

if [ -z "$DB" ]; then
	echo "keepassxc-autounlock: no database configured — set MANGO_KEEPASS_DB in mango/local.conf" >&2
	exit 1
fi

exec keepassxc "$DB"
