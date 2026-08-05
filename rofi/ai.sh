#!/usr/bin/env bash
# rofi script mode: a multi-turn AI chat with browsable history.
#
#   ai.sh --launch   spawn rofi with the chat keybindings (bound to Alt+I)
#   ai.sh            rofi script mode entry point
#   ai.sh test       assert the pure helpers against canned data
#
# The transcript is the listview: every message is wrapped to the window width
# and each wrapped line is its own row, so a long answer simply occupies more
# rows. rofi cannot do variable-height rows, and this gets the same result
# without fighting it.
#
# Enter sends what you typed (kb-accept-custom is rebound to Return), so typing
# is the primary action; Ctrl+Enter copies the highlighted message.
#
# The request runs detached: the typed message is stored and drawn immediately
# with a "···" placeholder, curl is spawned via `--fetch`, and the reply is
# reaped on the next invocation. rofi's script protocol has no timer, so the
# transcript does not refresh by itself — Alt+r does that. The win is that the
# window stays live (scroll, copy, switch chats) instead of freezing for the
# length of the call.
set -euo pipefail

DIR="${MANGO_AI_DIR:-$HOME/.local/share/rofi-ai}"
CHATS="$DIR/chats"
CURRENT="$DIR/current"
PENDING="$DIR/pending"  # "<chat id> <epoch>" while a request is in flight
REPLY_F="$DIR/reply"    # the worker's answer, renamed into place when complete
ERR_F="$DIR/err"        # ...or its error, same
KEYRING_SCRIPT="$HOME/.config/mango/scripts/keyring-lookup.sh"
SELF=$(readlink -f "$0")  # re-invoked as the detached worker, so resolve it once
MODEL="${MANGO_AI_MODEL:-deepseek/deepseek-r1-distill-llama-70b:free}"

MAX_CHATS=20   # keep this many conversations
MAX_TURNS=40   # ...and this many messages inside each one
WRAP=72        # characters per row at the 720px window width

# ---------------------------------------------------------------- primitives

# rofi's script protocol is line based: \n ends a row, \x1f separates a row from
# its options, and the whole thing is plain text.
opt() { printf '\0%s\x1f%s\n' "$1" "$2"; }
esc() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

# One message -> its rows. Role label on the first line, hanging indent after.
# Every row carries the same info index so any line copies the whole message.
render_msg() { # role, content, index
	local role=$1 body=$2 idx=$3 label
	case "$role" in
	user) label="You " ;;
	assistant) label="AI  " ;;
	*) label="··· " ;;
	esac
	# A row is "text NUL opt US value" — the NUL is what separates the visible
	# text from its options; getting that wrong prints "\x1finfo\x1f3" on screen.
	# awk cannot emit NUL portably, so it writes \034 and tr swaps it after.
	printf '%s\n' "$body" | fold -s -w "$WRAP" | sed 's/[[:space:]]*$//' |
		awk -v l="$label" -v i="$idx" '{
			if (NR == 1) printf "%s  %s\034info\037%s\n", l, $0, i
			else printf "      %s\034info\037%s\n", $0, i
		}' | tr '\034' '\000'
}

# Newest chat ids first.
chat_ids() { ls -1 "$CHATS" 2> /dev/null | sed 's/\.json$//' | sort -rn; }

# Drop everything past the newest MAX_CHATS conversations.
prune_chats() {
	chat_ids | tail -n +$((MAX_CHATS + 1)) | while read -r id; do
		rm -f "$CHATS/$id.json"
	done
}

# Keep a conversation from growing past MAX_TURNS messages.
trim() { jq --argjson n "$MAX_TURNS" 'if length > $n then .[-$n:] else . end'; }

# Position of the current chat, "3/12", for the caption line.
chat_pos() { # id
	chat_ids | sort -n | awk -v c="$1" '{ n = NR; if ($0 == c) p = NR } END { printf "%d/%d", p, n }'
}

# First user message, shortened, as the conversation's name.
chat_title() { # file
	jq -r 'map(select(.role == "user")) | first | .content // "empty chat"' "$1" 2> /dev/null |
		tr '\n' ' ' | cut -c1-48 | sed 's/[[:space:]]*$//'
}

# ---------------------------------------------------------------- state

new_chat() {
	local id
	id=$(date +%s)
	mkdir -p "$CHATS"
	[ -f "$CHATS/$id.json" ] || printf '[]\n' > "$CHATS/$id.json"
	printf '%s' "$id" > "$CURRENT"
	prune_chats
	printf '%s' "$id"
}

current_id() {
	local id
	id=$(cat "$CURRENT" 2> /dev/null || true)
	if [ -z "$id" ] || [ ! -f "$CHATS/$id.json" ]; then
		id=$(chat_ids | head -1)
	fi
	[ -n "$id" ] || id=$(new_chat)
	printf '%s' "$id"
}

# Step through the history, oldest to newest.
step_chat() { # +1 | -1
	local cur ids i n
	cur=$(current_id)
	mapfile -t ids < <(chat_ids | sort -n)
	n=${#ids[@]}
	for i in "${!ids[@]}"; do
		[ "${ids[$i]}" = "$cur" ] || continue
		i=$((i + $1))
		[ "$i" -lt 0 ] && i=0
		[ "$i" -ge "$n" ] && i=$((n - 1))
		printf '%s' "${ids[$i]}" > "$CURRENT"
		return
	done
}

# ---------------------------------------------------------------- pending

# Drop a trailing user message. A request that failed leaves the question it was
# sent with on screen and nothing to answer it, so it comes back off — same
# "a failed call leaves nothing half-sent" property the synchronous version had.
pop_user() { jq 'if (length > 0 and .[-1].role == "user") then .[:-1] else . end'; }

# curl is capped at 90s, so a marker older than this means the worker died
# without writing either file — otherwise the panel waits on it forever.
STALE=120

# Merge a finished request into its chat. Sets ERROR when the call failed.
# Runs on every invocation, which is what makes Alt+r (or any other keypress
# that re-invokes the script) the refresh.
reap() {
	[ -f "$PENDING" ] || return 0
	local id started file
	# `|| true`: read reports EOF on a file with no trailing newline, and under
	# set -e that would take the whole script down.
	read -r id started < "$PENDING" || true
	started=${started:-0}
	file="$CHATS/$id.json"
	[ -f "$file" ] || { rm -f "$PENDING" "$REPLY_F" "$ERR_F"; return 0; }

	if [ -f "$REPLY_F" ]; then
		jq --arg a "$(cat "$REPLY_F")" '. + [{role: "assistant", content: $a}]' "$file" |
			trim > "$file.tmp" && mv "$file.tmp" "$file"
	elif [ -f "$ERR_F" ]; then
		ERROR=$(cat "$ERR_F")
		pop_user < "$file" > "$file.tmp" && mv "$file.tmp" "$file"
	elif [ $(($(date +%s) - started)) -gt "$STALE" ]; then
		ERROR="request timed out"
		pop_user < "$file" > "$file.tmp" && mv "$file.tmp" "$file"
	else
		return 0 # still in flight
	fi
	rm -f "$PENDING" "$REPLY_F" "$ERR_F"
}

# ---------------------------------------------------------------- API

ask() { # chat-file, question -> assistant reply on stdout, error on stderr
	local file=$1 question=$2 key_json key payload answer
	key_json=$("$KEYRING_SCRIPT" 2> /dev/null || true)
	case "$key_json" in
	locked) echo "keyring locked — unlock KeePassXC first" >&2; return 1 ;;
	"not found" | "") echo "no keyring entry for application=mango" >&2; return 1 ;;
	esac
	key=$(jq -r '.apiKeys.openrouter // empty' <<< "$key_json" 2> /dev/null || true)
	[ -n "$key" ] || { echo "no OpenRouter key in the keyring entry" >&2; return 1; }

	# The whole conversation goes up, which is what makes follow-ups work.
	payload=$(jq -c --arg m "$MODEL" --arg q "$question" \
		'{model: $m, messages: (. + [{role: "user", content: $q}])}' "$file")
	answer=$(curl -sS --max-time 90 https://openrouter.ai/api/v1/chat/completions \
		-H "Authorization: Bearer $key" -H "Content-Type: application/json" \
		-d "$payload" 2> /dev/null |
		jq -r 'if .error then "!" + (.error.message // "unknown error")
		       else (.choices[0].message.content // "") end' 2> /dev/null)

	case "$answer" in
	"") echo "no response from $MODEL" >&2; return 1 ;;
	"!"*) printf '%s' "${answer#!}" >&2; return 1 ;;
	esac
	printf '%s' "$answer"
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = "test" ]; then
	DIR=$(mktemp -d); CHATS="$DIR/chats"; CURRENT="$DIR/current"; mkdir -p "$CHATS"
	PENDING="$DIR/pending"; REPLY_F="$DIR/reply"; ERR_F="$DIR/err"
	trap 'rm -rf "$DIR"' EXIT
	# Bash drops NUL bytes from $(...), so the separator is made visible first.
	rendered() { render_msg "$1" "$2" "$3" | tr '\000' '@'; }
	US='@'

	out=$(rendered user "hello there" 3)
	[ "$out" = "You   hello there${US}info"$'\x1f'"3" ] || { echo "render_msg wrong: $(printf '%q' "$out")"; exit 1; }

	# a long answer becomes several rows, all pointing at the same message
	long=$(printf 'word%.0s ' $(seq 1 60))
	rows=$(rendered assistant "$long" 7)
	[ "$(printf '%s\n' "$rows" | wc -l)" -ge 3 ] || { echo "long answer should wrap to several rows"; exit 1; }
	[ "$(printf '%s\n' "$rows" | grep -c "${US}info"$'\x1f'"7\$")" = "$(printf '%s\n' "$rows" | wc -l)" ] ||
		{ echo "every wrapped row must carry the message index"; exit 1; }
	printf '%s\n' "$rows" | head -1 | grep -q '^AI  ' || { echo "first row needs the role label"; exit 1; }
	printf '%s\n' "$rows" | tail -1 | grep -q '^      ' || { echo "continuation rows need the hanging indent"; exit 1; }

	# newlines inside a message must not break the one-row-per-line contract
	rows=$(rendered assistant $'first\nsecond' 1)
	[ "$(printf '%s\n' "$rows" | wc -l)" = 2 ] || { echo "embedded newlines wrong"; exit 1; }

	printf '[{"role":"user","content":"why"},{"role":"assistant","content":"because"}]' > "$CHATS/100.json"
	[ "$(chat_title "$CHATS/100.json")" = "why" ] || { echo "chat_title wrong"; exit 1; }
	printf '[]' > "$CHATS/101.json"
	[ "$(chat_title "$CHATS/101.json")" = "empty chat" ] || { echo "empty chat_title wrong"; exit 1; }

	# --- pending / reap -------------------------------------------------
	[ "$(jq -c . <<< '[{"role":"user","content":"q"}]' | pop_user)" = "[]" ] ||
		{ echo "pop_user must drop a trailing question"; exit 1; }
	[ "$(jq -c . <<< '[{"role":"user","content":"q"},{"role":"assistant","content":"a"}]' | pop_user | jq -c 'length')" = "2" ] ||
		{ echo "pop_user must leave an answered turn alone"; exit 1; }
	[ "$(jq -c . <<< '[]' | pop_user)" = "[]" ] || { echo "pop_user on empty wrong"; exit 1; }

	# a question waiting on an answer stays put and reports nothing
	arm() { # started-epoch
		printf '[{"role":"user","content":"q"}]' > "$CHATS/500.json"
		printf '500 %s\n' "$1" > "$PENDING"
		rm -f "$REPLY_F" "$ERR_F"
		ERROR=""
	}
	arm "$(date +%s)"; reap
	[ -f "$PENDING" ] || { echo "reap dropped a request still in flight"; exit 1; }
	[ -z "$ERROR" ] || { echo "in-flight request should not report an error"; exit 1; }

	# a landed reply is appended and the marker cleared
	arm "$(date +%s)"; printf 'because' > "$REPLY_F"; reap
	[ ! -f "$PENDING" ] || { echo "reap left the marker behind"; exit 1; }
	[ "$(jq -r '.[-1].role + ":" + .[-1].content' "$CHATS/500.json")" = "assistant:because" ] ||
		{ echo "reply not merged: $(jq -c . "$CHATS/500.json")"; exit 1; }

	# a failed call reports the error and takes the question back off
	arm "$(date +%s)"; printf 'keyring locked' > "$ERR_F"; reap
	[ "$ERROR" = "keyring locked" ] || { echo "reap lost the error: '$ERROR'"; exit 1; }
	[ "$(jq -c . "$CHATS/500.json")" = "[]" ] || { echo "failed question was not rolled back"; exit 1; }
	[ ! -f "$PENDING" ] || { echo "reap left the marker after an error"; exit 1; }

	# a worker that died without writing anything must not wedge the panel
	arm "$(($(date +%s) - STALE - 1))"; reap
	[ "$ERROR" = "request timed out" ] || { echo "stale request not timed out: '$ERROR'"; exit 1; }
	[ "$(jq -c . "$CHATS/500.json")" = "[]" ] || { echo "stale question was not rolled back"; exit 1; }

	# a marker pointing at a deleted chat is cleaned up, not crashed on
	arm "$(date +%s)"; rm -f "$CHATS/500.json"; reap
	[ ! -f "$PENDING" ] || { echo "marker for a deleted chat survived"; exit 1; }
	rm -f "$CHATS/500.json"
	ERROR=""

	MAX_TURNS=2
	[ "$(jq -c . <<< '[1,2,3,4]' | trim | jq -c .)" = "[3,4]" ] || { echo "trim wrong"; exit 1; }
	[ "$(jq -c . <<< '[1]' | trim | jq -c .)" = "[1]" ] || { echo "trim must leave short chats alone"; exit 1; }

	MAX_CHATS=2
	for id in 200 201 202 203; do printf '[]' > "$CHATS/$id.json"; done
	prune_chats
	[ "$(chat_ids | tr '\n' ' ')" = "203 202 " ] || { echo "prune_chats wrong: $(chat_ids | tr '\n' ' ')"; exit 1; }

	printf '203' > "$CURRENT"
	[ "$(chat_pos 203)" = "2/2" ] || { echo "chat_pos wrong: $(chat_pos 203)"; exit 1; }
	step_chat -1
	[ "$(cat "$CURRENT")" = "202" ] || { echo "step back wrong: $(cat "$CURRENT")"; exit 1; }
	step_chat -1
	[ "$(cat "$CURRENT")" = "202" ] || { echo "step back must clamp at the oldest"; exit 1; }
	step_chat +1
	[ "$(cat "$CURRENT")" = "203" ] || { echo "step forward wrong"; exit 1; }
	step_chat +1
	[ "$(cat "$CURRENT")" = "203" ] || { echo "step forward must clamp at the newest"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- worker

# Spawned detached by the mode below. $2 is a snapshot of the conversation as it
# stood *before* this turn — ask() appends the question itself, so handing it the
# live file would send the question twice.
#
# Both outputs are renamed into place so reap() can never read a half-written
# file: the two exist precisely to be raced against.
if [ "${1:-}" = "--fetch" ]; then
	# Scratch files are pid-suffixed: the mode below only ever runs one worker,
	# but a stray second one must not rename another's half-written file away.
	if reply=$(ask "$2" "$3" 2> "$ERR_F.$$"); then
		printf '%s' "$reply" > "$REPLY_F.$$" && mv "$REPLY_F.$$" "$REPLY_F"
	else
		mv "$ERR_F.$$" "$ERR_F"
	fi
	rm -f "$ERR_F.$$" "$REPLY_F.$$" "$2"
	exit 0
fi

# ---------------------------------------------------------------- launcher

if [ "${1:-}" = "--launch" ]; then
	# Enter sends the typed message and Ctrl+Enter copies the highlighted one:
	# in a chat, typing is the primary action, not picking from the list.
	#
	# History is Alt+[ / Alt+] because mango grabs Alt+Left/Right (focusdir) —
	# a compositor bind never reaches rofi. Check mango/config.conf before
	# picking any Alt combo here.
	exec rofi -show ai -theme "$HOME/.config/rofi/ai.rasi" \
		-kb-accept-custom "Return" \
		-kb-accept-entry "Control+Return" \
		-kb-custom-1 "Alt+n" \
		-kb-custom-2 "Alt+d" \
		-kb-custom-3 "Alt+bracketleft" \
		-kb-custom-4 "Alt+bracketright" \
		-kb-custom-5 "Alt+Shift+d" \
		-kb-custom-6 "Alt+r"
fi

# ---------------------------------------------------------------- mode

mkdir -p "$CHATS"
ERROR=""

# Pick up a finished request first, so every keypress that re-invokes the script
# doubles as a refresh — Alt+r is just the one that does nothing else.
reap

case "${ROFI_RETV:-0}" in
1) # Ctrl+Enter on a row — ROFI_INFO is the index of the message it came from
	if [ -n "${ROFI_INFO:-}" ]; then
		jq -r --argjson i "$ROFI_INFO" '.[$i].content // ""' "$CHATS/$(current_id).json" | wl-copy
	fi
	;;
2) # typed text
	if [ -n "${1:-}" ]; then
		if [ -f "$PENDING" ]; then
			# One in flight at a time: a second question would race the first
			# one's reply into the wrong place in the transcript.
			ERROR="still waiting on the last question — Alt+r to check"
		else
			id=$(current_id)
			file="$CHATS/$id.json"
			# ask() appends the question to whatever conversation it is handed,
			# so the worker gets a snapshot taken *before* this turn while the
			# live file gets the question immediately, to draw.
			snap="$DIR/payload.$id.json"
			cp "$file" "$snap"
			jq --arg q "$1" '. + [{role: "user", content: $q}]' "$file" |
				trim > "$file.tmp" && mv "$file.tmp" "$file"
			rm -f "$REPLY_F" "$ERR_F"
			printf '%s %s\n' "$id" "$(date +%s)" > "$PENDING"
			# Detached and fully redirected: rofi will not draw until this
			# script exits, and an inherited stdout would hold it open.
			setsid "$SELF" --fetch "$snap" "$1" > /dev/null 2>&1 &
		fi
	fi
	;;
10) new_chat > /dev/null ;;
11)
	printf '[]\n' > "$CHATS/$(current_id).json"
	rm -f "$PENDING" "$REPLY_F" "$ERR_F"
	;;
12) step_chat -1 ;;
13) step_chat +1 ;;
14)
	rm -rf "$CHATS" "$CURRENT" "$PENDING" "$REPLY_F" "$ERR_F"
	mkdir -p "$CHATS"
	;;
15) : ;; # Alt+r — the reap above is the whole point
esac

ID=$(current_id)
FILE="$CHATS/$ID.json"
[ -f "$FILE" ] || printf '[]\n' > "$FILE"
COUNT=$(jq 'length' "$FILE")

opt use-hot-keys true
opt keep-selection true

# Is the in-flight request, if any, the one belonging to the chat on screen?
# Alt+[/] can move off it while it runs, and the placeholder must not follow.
PENDING_HERE=""
if [ -f "$PENDING" ]; then
	read -r pid _ < "$PENDING" || true
	[ "$pid" = "$ID" ] && PENDING_HERE=1
fi

if [ -n "$ERROR" ]; then
	opt message "$(printf '%s' "⚠  $ERROR" | esc)"
elif [ -n "$PENDING_HERE" ]; then
	opt message "$(printf 'asking %s …   —   Alt+r refreshes' "${MODEL##*/}" | esc)"
elif [ "$COUNT" = 0 ]; then
	opt message "Type and press Enter.   Alt+n new chat · Alt+[/] history · Alt+Shift+D wipe"
else
	opt message "$(printf 'chat %s · %s   —   Ctrl+Enter copies · Alt+d clears · Alt+[/] history' \
		"$(chat_pos "$ID")" "$(chat_title "$FILE")" | esc)"
fi

# Rows are built into a file rather than a variable: they contain NUL bytes
# (the row/option separator) and $(...) silently drops those.
BUF=$(mktemp)
trap 'rm -f "$BUF"' EXIT
i=0
jq -r '.[] | .role + " " + (.content | @base64)' "$FILE" | while read -r role b64; do
	render_msg "$role" "$(printf '%s' "$b64" | base64 -d)" "$i" >> "$BUF"
	i=$((i + 1))
done
# The placeholder sits past the last real message, so its index copies nothing
# on Ctrl+Enter (jq returns null for an out-of-range element, and that is fine).
[ -n "$PENDING_HERE" ] && render_msg pending "thinking …" "$COUNT" >> "$BUF"
ROWS=$(wc -l < "$BUF")

# Park the highlight on the newest line so the view opens at the bottom of the
# conversation rather than the top.
[ "$ROWS" -gt 0 ] && opt new-selection $((ROWS - 1))
cat "$BUF"
exit 0
