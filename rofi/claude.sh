#!/usr/bin/env bash
# rofi picker: start a Claude Code session in one of your project directories.
#
#   claude.sh        pick a directory, spawn `kitty --directory <dir> claude`
#   claude.sh test   assert the row builder against a canned project tree
#
# Bound to Alt+C. Every press opens a new kitty; there is no scratchpad.
#
# The list needs a path and a timestamp, and each half comes from a different
# file. ~/.claude.json holds exact absolute paths under .projects, but no
# recency. ~/.claude/projects/<dir> has the mtime, but its name replaces every
# non-alphanumeric character with "-", so "erpnext_mauritius" and
# "erpnext-mauritius" mangle to the same dir and the name cannot be decoded
# back. So the mangle runs forward: take the JSON path, mangle it, stat it.
set -euo pipefail

CLAUDE_JSON="$HOME/.claude.json"
PROJECTS="$HOME/.claude/projects"
THEME="$HOME/.config/rofi/window.rasi"

# -> one directory per line, most recently used first, $HOME shown as ~.
# Paths that no longer exist on disk are dropped; a project with no session
# directory sorts last (mtime 0) instead of disappearing.
rows() {
	jq -r '.projects // {} | keys[]' "$CLAUDE_JSON" | while IFS= read -r path; do
		[[ -d $path ]] || continue
		ts=$(stat -c %Y "$PROJECTS/$(printf '%s' "$path" | tr -c 'A-Za-z0-9' '-')" 2>/dev/null) || ts=0
		printf '%s\t%s\n' "$ts" "${path/#$HOME/\~}"
	done | sort -rn | cut -f2
}

# ---------------------------------------------------------------- selftest

if [[ ${1:-} == test ]]; then
	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' EXIT
	mkdir -p "$tmp/a_b" "$tmp/c" "$tmp/projects"
	# expected mangling, spelled out here rather than reusing rows()' tr
	mangled="${tmp//[!A-Za-z0-9]/-}"
	mkdir -p "$tmp/projects/$mangled-a-b" "$tmp/projects/$mangled-c"
	touch -d 2020-01-01 "$tmp/projects/$mangled-c"
	touch -d 2021-01-01 "$tmp/projects/$mangled-a-b"
	printf '{"projects":{"%s":{},"%s":{},"%s":{}}}' "$tmp/c" "$tmp/a_b" "$tmp/gone" >"$tmp/json"

	CLAUDE_JSON="$tmp/json"
	PROJECTS="$tmp/projects"
	mapfile -t out < <(rows)

	[[ ${#out[@]} == 2 ]] || { echo "a path that no longer exists must be dropped"; exit 1; }
	[[ ${out[0]} == "$tmp/a_b" ]] || { echo "newest first, and _ must match its - dir: ${out[0]}"; exit 1; }
	[[ ${out[1]} == "$tmp/c" ]] || { echo "older project must come second: ${out[1]}"; exit 1; }
	exit 0
fi

# ---------------------------------------------------------------- launch

sel=$(rows | rofi -dmenu -i -p "claude" -theme "$THEME") || exit 0
dir="${sel/#\~/$HOME}"
[[ -d $dir ]] || exit 0
exec kitty --directory "$dir" claude
