#!/bin/sh
# Value source for the status panel sliders (genconfig.rs status_slider_row).
# ironbar runs it in `watch` mode: each stdout line sets the slider value.
#
#   slider-watch.sh volume|mic|brightness
#   slider-watch.sh test
#
# The script prints the value once, then again only on a real change event.
# It does not poll, so a closed panel costs no wakeups. A poll-mode `value`
# ran every 5 s on every bar, and it put a dragged brightness slider back
# to the value from the panel open (nothing refreshed bri_pct).
#
# Events:
#   volume, mic  `pactl subscribe` lines "Event 'change' on sink #N" and
#                "... on source #N". A "sink-input" line (a stream) does
#                not match, so playing audio does not wake the script.
#   brightness   the backlight uevent. The kernel sends a "change" uevent
#                on each sysfs write, so the brightness keys and the slider
#                itself both report (checked 2026-10-03 on intel_backlight).
#
# ironbar stops a watch script with SIGKILL on this shell only. Without
# `setpriv --pdeathsig`, pactl and udevadm stay alive until their next
# event (seen in a test on 2026-10-03). With it they get SIGTERM when this
# shell dies.
set -u

pct_volume() {
	wpctl get-volume "$1" 2>/dev/null | awk '{ printf "%d\n", $2 * 100 + 0.5 }'
}

pct_brightness() {
	brightnessctl -m 2>/dev/null | awk -F, '{ sub(/%/, "", $4); print $4 }'
}

# Print the value when it differs from the last one. ironbar parses every
# line, so a repeat costs a redraw for no change. Returns 1 when nothing
# was printed.
emit() {
	v=$1
	[ -n "$v" ] || return 1
	[ "$v" = "${last:-}" ] && return 1
	last=$v
	printf '%s\n' "$v"
}

watch_audio() {
	node=$1 kind=$2
	emit "$(pct_volume "$node")"
	setpriv --pdeathsig TERM pactl subscribe 2>/dev/null | while read -r line; do
		case $line in
		*" on $kind #"*) emit "$(pct_volume "$node")" ;;
		esac
	done
}

# The panel's number label reads the ironvar bri_pct. mango-bard sets it
# only on panel open, so this watcher keeps it live: a drag, a brightness
# key or the bar scroll all change the backlight and land here. Volume and
# mic need no such write: mango-bard sets vol_pct/mic_pct from pactl.
emit_brightness() {
	emit "$1" && ironbar var set bri_pct "$1" >/dev/null 2>&1
	return 0
}

watch_brightness() {
	emit_brightness "$(pct_brightness)"
	setpriv --pdeathsig TERM udevadm monitor --udev --subsystem-match=backlight 2>/dev/null | while read -r line; do
		case $line in
		UDEV*change*) emit_brightness "$(pct_brightness)" ;;
		esac
	done
}

selftest() {
	# The event filter: only an exact sink/source line matches.
	for l in "Event 'change' on sink #56" "Event 'change' on sink-input #90" "Event 'new' on source #3"; do
		case $l in
		*" on sink #"*) printf 'sink   %s\n' "$l" ;;
		*" on source #"*) printf 'source %s\n' "$l" ;;
		*) printf 'skip   %s\n' "$l" ;;
		esac
	done | {
		read -r a && read -r b && read -r c
		[ "${a%% *}" = sink ] && [ "${b%% *}" = skip ] && [ "${c%% *}" = source ] ||
			{ echo "FAIL filter: $a / $b / $c"; exit 1; }
	}
	# emit drops a repeat and an empty value.
	out=$(last=; emit 5; emit 5 || :; emit '' || :; emit 6)
	[ "$out" = "5
6" ] || { echo "FAIL emit: $out"; exit 1; }
	echo ok
}

case "${1:-}" in
volume) watch_audio @DEFAULT_AUDIO_SINK@ sink ;;
mic) watch_audio @DEFAULT_AUDIO_SOURCE@ source ;;
brightness) watch_brightness ;;
test) selftest ;;
*)
	echo "usage: $0 volume|mic|brightness|test" >&2
	exit 2
	;;
esac
