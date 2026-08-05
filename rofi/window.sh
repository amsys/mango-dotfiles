#!/usr/bin/env bash
# rofi script mode: jump to any window, on any tag.
#
#   window.sh --launch   spawn rofi with this mode (bound to Alt+grave)
#   window.sh            rofi script-mode callback
#   window.sh test       assert the row builder against canned mmsg output
#
# rofi-wayland has no usable `window` mode — the upstream one is X11/EWMH only —
# so the list comes from mango's IPC instead.
#
# Selecting a row is a single `mmsg dispatch focusid client,<id>`: focusid runs
# client_active(), which views the window's tag, un-minimizes it if it was
# minimized, and focuses it. Minimized windows are therefore listed on purpose —
# this is the only way back to a specific one (SUPER+Shift+I restores blindly).
set -euo pipefail

THEME="$HOME/.config/rofi/window.rasi"

opt() { printf '\0%s\x1f%s\n' "$1" "$2"; }

# all-clients JSON on stdin -> one rofi row per window, "<text>\0info\x1f<id>".
#
# jq writes \034 where the NUL belongs and tr swaps it after: jq will not emit a
# NUL byte, and the NUL is what separates a row's text from its options. Same
# dance as ai.sh, for the same reason.
#
# Sorted by tag then id so the list reads in tag order and a window keeps its
# position between invocations — the id is creation order, so new windows land
# at the bottom of their tag rather than shuffling the rest.
rows() {
	jq -r '
		[ .clients[] | {
			id,
			tag: (.tags[0] // 0),
			appid,
			title,
			focused: .is_focused,
			min: .is_minimized
		} ]
		| sort_by(.tag, .id)
		| .[]
		| (if .focused then "●" else "·" end) as $mark
		| (if .min then "[min] " else "" end) as $state
		| "\($mark) \(.tag)  \(.appid)  —  \($state)\(.title | gsub("\n"; " "))"
		  + "info\(.id)"
	' | tr '\034' '\000'
}

# ---------------------------------------------------------------- selftest

if [[ ${1:-} == test ]]; then
	# Bash drops NUL from $(...), so the separator is made visible first.
	canned='{"clients":[
		{"id":9,"tags":[3],"appid":"firefox","title":"Docs","is_focused":false,"is_minimized":false},
		{"id":4,"tags":[1],"appid":"kitty","title":"shell","is_focused":true,"is_minimized":false},
		{"id":7,"tags":[1],"appid":"dolphin","title":"Home","is_focused":false,"is_minimized":true}
	]}'
	out=$(rows <<< "$canned" | tr '\000' '@')

	[[ $(wc -l <<< "$out") == 3 ]] || { echo "expected one row per window"; exit 1; }

	# tag 1 before tag 3, and within tag 1 id 4 before id 7
	[[ $(sed -n 1p <<< "$out") == *"kitty"* ]] || { echo "sort_by(tag,id) wrong: $(sed -n 1p <<< "$out")"; exit 1; }
	[[ $(sed -n 2p <<< "$out") == *"dolphin"* ]] || { echo "id tiebreak wrong: $(sed -n 2p <<< "$out")"; exit 1; }
	[[ $(sed -n 3p <<< "$out") == *"firefox"* ]] || { echo "tag order wrong: $(sed -n 3p <<< "$out")"; exit 1; }

	# every row carries its client id, and that id is what focusid gets
	[[ $(sed -n 1p <<< "$out") == *"@info"$'\x1f'"4" ]] || { echo "info must end the row with the id"; exit 1; }
	[[ $(sed -n 3p <<< "$out") == *"@info"$'\x1f'"9" ]] || { echo "wrong id on the last row"; exit 1; }

	# markers: exactly one focused, minimized flagged
	[[ $(grep -c '^●' <<< "$out") == 1 ]] || { echo "focused marker wrong"; exit 1; }
	[[ $(sed -n 1p <<< "$out") == "● 1  kitty"* ]] || { echo "focused row wrong: $(sed -n 1p <<< "$out")"; exit 1; }
	[[ $(sed -n 2p <<< "$out") == *"—  [min] Home"* ]] || { echo "minimized not flagged"; exit 1; }

	# a title with a newline must not become two rows
	one='{"clients":[{"id":1,"tags":[1],"appid":"x","title":"a\nb","is_focused":false,"is_minimized":false}]}'
	[[ $(rows <<< "$one" | tr '\000' '@' | wc -l) == 1 ]] || { echo "newline in title split the row"; exit 1; }

	# no windows is a valid state, not an error
	[[ -z $(rows <<< '{"clients":[]}') ]] || { echo "empty client list should print nothing"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- launcher

if [[ ${1:-} == --launch ]]; then
	# No -kb-custom here: Enter is the only action, so unlike ai.sh and
	# clipboard.sh this could be a bare `rofi -show window`. It keeps the
	# --launch shape anyway so every rofi bind in config.conf looks the same.
	exec rofi -show window -theme "$THEME"
fi

# ---------------------------------------------------------------- mode

if [[ ${ROFI_RETV:-0} == 1 ]]; then
	# ROFI_INFO is only promised on a selection, which is all we have.
	[[ -n ${ROFI_INFO:-} ]] && mmsg dispatch focusid client,"$ROFI_INFO"
	exit 0
fi

# A dead compositor socket means no windows, not a broken menu.
LIST=$(mmsg get all-clients 2> /dev/null || printf '{"clients":[]}')

opt no-custom true
opt use-hot-keys true
opt message "$(jq '.clients | length' <<< "$LIST") windows   —   Enter jumps to it, minimized included"

# Straight to stdout: the rows carry NUL bytes and $(...) drops those.
rows <<< "$LIST"
