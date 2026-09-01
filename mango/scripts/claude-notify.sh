#!/usr/bin/env bash
# Claude Code Notification hook: reads the hook JSON from stdin and shows a
# mako notification summarised with the repo the session is in, instead of
# the generic "Claude Code" summary with no project name.
#
# Two modes, selected by argument:
#   (none)  — run as the hook; read JSON from stdin, show the notification.
#   click   — run by mako's on-button-left (matugen/templates/mako/config);
#             focus the window that sent the notification $2, then dismiss.
#
# Repo label rule kept in sync with kitty/repo-title.py — see that file if
# you change this.
#
# Wired from ~/.claude/settings.json's Notification hook, not from here:
# settings.json lives outside this repo by design (RULES.md).
set -uo pipefail

STATE_DIR="${XDG_RUNTIME_DIR:-/tmp}/claude-notify"

# --- click mode (run by mako's on-button-left) ---------------------------
if [ "${1:-}" = click ]; then
	cid="$(cat "$STATE_DIR/n${2:-}" 2> /dev/null)" || exit 0
	[ -n "$cid" ] || exit 0
	# focusid views the client's tag, restores it and focuses it — same
	# single dispatch as rofi/window.sh and system/fprint-notify.
	timeout 2 mmsg dispatch focusid client,"$cid" > /dev/null 2>&1
	exec timeout 2 makoctl dismiss -n "${2:-}" > /dev/null 2>&1
fi

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
mkdir -p "$STATE_DIR" 2> /dev/null
STATE="$STATE_DIR/$session"
replace_id="$(cat "$STATE" 2> /dev/null || true)"

args=(-a claude -p -r "${replace_id:-0}")
[ "$blocked" = 1 ] && args+=(-c blocked)

nid="$(timeout 3 notify-send "${args[@]}" "$summary" "$message" 2> /dev/null)" || exit 0
[ -n "$nid" ] && printf '%s\n' "$nid" > "$STATE" 2> /dev/null

# The window to jump to on click: climb the process tree (hook -> claude ->
# shell -> kitty) until a pid owns a mango client.
# ponytail: breaks if claude runs under tmux/zellij — the tree then leads to
# the multiplexer server, not the terminal. Match on the repo-title prefix
# if that ever becomes a real setup.
if [ -n "$nid" ]; then
	cid=""
	clients="$(timeout 2 mmsg get all-clients 2> /dev/null)"
	pid=$PPID
	for _ in {1..15}; do
		[ "${pid:-0}" -gt 1 ] || break
		cid="$(jq -r --argjson p "$pid" \
			'first(.clients[] | select(.pid == $p)) | .id // empty' \
			<<< "$clients" 2> /dev/null)"
		[ -n "$cid" ] && break
		# ppid is the 2nd field after the ')' that ends comm.
		pid="$(sed 's/^.*) //' "/proc/$pid/stat" 2> /dev/null | cut -d' ' -f2)"
	done
	[ -n "$cid" ] && printf '%s\n' "$cid" > "$STATE_DIR/n$nid" 2> /dev/null
fi

exit 0
