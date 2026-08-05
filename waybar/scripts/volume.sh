#!/bin/bash
# Volume indicator for waybar, with a sink / mic / per-app tooltip.
#
# Replaces the built-in `pulseaudio` module for the same reason net.sh replaced
# `network`: waybar's tooltip can only print waybar's own placeholders, and none
# of them expose the active port, the card profile, the mic, or what is actually
# playing. The bar text itself is unchanged.
#
#   volume.sh          custom/volume exec — emit JSON
#   volume.sh --watch  custom/volwatch exec — block on pactl subscribe, signal
#   volume.sh test     assert the parsers against canned input
set -u

. "$(dirname "$0")/tooltip.sh"
. "$(dirname "$0")/watch.sh"

SIGNAL=21

IC_OUT='󰓃'   # md-speaker             U+F04C3
IC_MIC='󰍬'   # md-microphone          U+F0368
IC_APP='󰝚'   # md-music-note          U+F075A

# ---------------------------------------------------------------- primitives

# The bar glyph, matching the ramp waybar's format-icons used.
ic_vol() { # pct, muted
	[ "$2" = true ] && { printf '󰝟'; return; }
	if [ "$1" -eq 0 ]; then printf '󰕿'
	elif [ "$1" -lt 34 ]; then printf '󰕿'
	elif [ "$1" -lt 67 ]; then printf '󰖀'
	else printf '󰕾'
	fi
}

# pactl -f json is the whole reason this script is short: the text output puts
# the volume, the port and the properties in three different shapes and none of
# them survive a device with a comma in its description.
#
# Emits one tab-separated line: pct, muted, label, port, profile.
dev_line() { # "sinks"|"sources", default-name
	pactl -f json list "$1" 2> /dev/null | jq -r --arg d "$2" '
		(map(select(.name == $d)) | first) // empty |
		[ ((.volume | to_entries[0].value.value_percent) | rtrimstr("%")),
		  (.mute | tostring),
		  (.properties["node.nick"] // .description // .name),
		  ((.active_port // "") | sub("^\\[(In|Out)\\] "; "")),
		  (.properties["device.profile.description"] // "")
		] | @tsv'
}

# Streams currently playing, loudest first. media.name is the track/tab title
# where an app bothers to set it, which is more useful than the app name alone.
app_lines() {
	pactl -f json list sink-inputs 2> /dev/null | jq -r '
		map(select(.corked | not)) |
		sort_by(-((.volume | to_entries[0].value.value_percent) | rtrimstr("%") | tonumber)) |
		.[] | [ ((.volume | to_entries[0].value.value_percent) | rtrimstr("%")),
		        (.mute | tostring),
		        (.properties["application.name"] // "?"),
		        (.properties["media.name"] // "")
		      ] | @tsv'
}

# ---------------------------------------------------------------- watch

if [ "${1:-}" = "--watch" ]; then
	# Continuous module: empty text = hidden, waybar owns the lifecycle, and
	# restart-interval respawns this if pipewire-pulse restarts under it.
	echo
	# pactl reports client connect/disconnect too — every `wpctl set-volume`
	# invocation is one — and refreshing on those would be pure noise.
	watch_match() { case "$1" in *" on sink"* | *" on source"* | *" on server"*) return 0 ;; *) return 1 ;; esac; }
	watch_loop "pactl subscribe"
	exit 0
fi

# ---------------------------------------------------------------- selftest

if [ "${1:-}" = "test" ]; then
	[ "$(ic_vol 0 false)" = '󰕿' ] || { echo "ic_vol 0 wrong"; exit 1; }
	[ "$(ic_vol 50 false)" = '󰖀' ] || { echo "ic_vol 50 wrong"; exit 1; }
	[ "$(ic_vol 90 false)" = '󰕾' ] || { echo "ic_vol 90 wrong"; exit 1; }
	[ "$(ic_vol 90 true)" = '󰝟' ] || { echo "mute icon wrong"; exit 1; }
	# The two parsers must survive a real server: no output is fine (an empty
	# sink list is legitimate), garbage or a jq error is not.
	OUT=$(dev_line sinks "$(pactl get-default-sink 2> /dev/null)") || { echo "dev_line failed"; exit 1; }
	case "$OUT" in
	'' | *"	"*) ;;
	*) echo "dev_line not tab-separated: $OUT"; exit 1 ;;
	esac
	app_lines > /dev/null || { echo "app_lines failed"; exit 1; }
	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- module

SINK=$(pactl get-default-sink 2> /dev/null || true)
OUT=$(dev_line sinks "$SINK")

if [ -z "$OUT" ]; then
	printf '{"text":"","class":"absent","tooltip":"No audio sink"}\n'
	exit 0
fi

VOL=$(printf '%s' "$OUT" | cut -f1)
MUTE=$(printf '%s' "$OUT" | cut -f2)
LABEL=$(printf '%s' "$OUT" | cut -f3)
PORT=$(printf '%s' "$OUT" | cut -f4)
PROFILE=$(printf '%s' "$OUT" | cut -f5)

CLASS=$([ "$MUTE" = true ] && echo muted || echo normal)
[ "$MUTE" != true ] && [ "$VOL" -gt 100 ] && CLASS=loud
TEXT="$(barico "$(ic_vol "$VOL" "$MUTE")") ${VOL}%"

TIP=$(
	title "Volume"
	rule 38

	sect "$IC_OUT" "Output"
	# Grade upward: quiet is fine, over 100% is software gain and clips.
	row "$(printf '%3s%%  %s' "$VOL" "$(bar "$VOL" "$(grade "$VOL" 101 130)")")"
	row "$(printf '%s' "$LABEL" | esc)$([ "$MUTE" = true ] && printf '  ·  %s' "$(bad muted)")"
	# On this machine's HiFi card the nick, the port and the profile description
	# are all literally "Speaker" — printing all three reads as a bug. Drop
	# whichever repeat what the row above already said.
	DETAIL=$(printf '%s\n%s\n' "$PORT" "$PROFILE" | awk -v l="$LABEL" '
		$0 != "" && $0 != l && !seen[$0]++ { s = s (s ? "  ·  " : "") $0 } END { print s }')
	[ -n "$DETAIL" ] && dim "$(printf '%s' "$DETAIL" | esc)"

	SRC=$(pactl get-default-source 2> /dev/null || true)
	MIC=$(dev_line sources "$SRC")
	if [ -n "$MIC" ]; then
		MVOL=$(printf '%s' "$MIC" | cut -f1)
		MMUTE=$(printf '%s' "$MIC" | cut -f2)
		sect "$IC_MIC" "Microphone"
		row "$(printf '%3s%%  %s' "$MVOL" "$(bar "$MVOL" "$([ "$MMUTE" = true ] && echo "$C_EMPTY" || echo "$C_GOOD")")")"
		row "$(printf '%s' "$MIC" | cut -f3 | esc)$([ "$MMUTE" = true ] && printf '  ·  %s' "$(good muted)")"
	fi

	APPS=$(app_lines)
	sect "$IC_APP" "Playing"
	if [ -z "$APPS" ]; then
		dim "nothing"
	else
		printf '%s\n' "$APPS" | head -5 | while IFS="$(printf '\t')" read -r av am an at; do
			row "$(printf '%s %4s%%  %s' \
				"$(bar "$av" "$([ "$am" = true ] && echo "$C_EMPTY" || echo "$C_GOOD")" 10)" \
				"$av" "$(printf '%s' "$an" | esc)")"
			[ -n "$at" ] && [ "$at" != "$an" ] && dim "$(printf '%s' "$at" | cut -c1-44 | esc)"
		done
	fi
)

emit "$CLASS" "$TEXT" "$TIP"
