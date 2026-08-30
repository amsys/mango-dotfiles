#!/usr/bin/env bash
# Claude Code Notification hook: reads the hook JSON from stdin and shows a
# mako notification summarised with the repo the session is in, instead of
# the generic "Claude Code" summary with no project name.
#
# Repo label rule kept in sync with kitty/repo-title.py — see that file if
# you change this.
#
# Wired from ~/.claude/settings.json's Notification hook, not from here:
# settings.json lives outside this repo by design (RULES.md).
set -uo pipefail

input="$(cat)"

cwd="$(jq -r '.cwd // empty' <<< "$input")"
message="$(jq -r '.message // empty' <<< "$input")"
ntype="$(jq -r '.notification_type // empty' <<< "$input")"
session="$(jq -r '.session_id // "default"' <<< "$input")"

[ -n "$message" ] || exit 0

repo_label() { # cwd -> label on stdout, empty if not under ~/work
	local rel="${1#"$HOME"/work/}"
	[ "$rel" = "$1" ] && return 0
	local a b c
	IFS=/ read -r a b c _ <<< "$rel"
	if [ "$b" = src ] && [ -n "$c" ]; then printf '%s/%s\n' "$a" "$c"
	else printf '%s\n' "$a"
	fi
}

summary="$(repo_label "$cwd")"
[ -n "$summary" ] || summary="$(basename "${cwd:-unknown}")"

# Types where Claude is stopped, waiting on the user — the ones worth
# pinning open rather than letting the default 3s timeout hide them.
blocked=0
case "$ntype" in
permission_prompt | elicitation_dialog | elicitation_url_dialog | agent_needs_input | worker_permission_prompt)
	blocked=1
	;;
esac

# Replace this session's own last notification instead of stacking a new
# one, same -p/-r id-file pattern as system/fprint-notify/fprint-notify.sh.
STATE_DIR="${XDG_RUNTIME_DIR:-/tmp}/claude-notify"
mkdir -p "$STATE_DIR" 2> /dev/null
STATE="$STATE_DIR/$session"
replace_id="$(cat "$STATE" 2> /dev/null || true)"

args=(-a claude -p -r "${replace_id:-0}")
[ "$blocked" = 1 ] && args+=(-c blocked)

nid="$(timeout 3 notify-send "${args[@]}" "$summary" "$message" 2> /dev/null)" || exit 0
[ -n "$nid" ] && printf '%s\n' "$nid" > "$STATE" 2> /dev/null

exit 0
