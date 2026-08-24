#!/usr/bin/env bash
# Calendar-app launcher for the date pill's click handler.
#
# Everything else this script used to do — the clock/date modules, the world
# clock and calendar tooltips — moved to mango-bard's clock.rs at the T8
# cutover (see IRONBAR.md). genconfig.rs builds both pills as var-driven with
# no `exec` key, so this is the only surface still reached from the bar:
# genconfig.rs's `date` pill on_click_left.
#
#   clock.sh --calendar   focus/open the calendar app
set -u

# What the date click opens. Thunderbird remotes -calendar into a running
# instance (switches its tab) rather than starting a second process.
CAL_CMD="${MANGO_CALENDAR_CMD:-thunderbird -calendar}"
CAL_APPID="${MANGO_CALENDAR_APPID:-org.mozilla.Thunderbird}"

case "${1:-}" in
--calendar)
	# `focusid` is mango's client_active(): it switches to the window's tag and
	# monitor and un-minimizes before focusing, so there is no tag maths here.
	# With nothing running there is no id to focus and mango focuses the new
	# window anyway, which is why the guard is the whole branch.
	id=$(mmsg get all-clients 2> /dev/null \
		| jq -r --arg a "$CAL_APPID" 'first(.clients[] | select(.appid == $a) | .id) // empty')
	[ -n "$id" ] && mmsg dispatch focusid client,"$id" 2> /dev/null
	setsid sh -c "$CAL_CMD" > /dev/null 2>&1 &
	;;
esac
