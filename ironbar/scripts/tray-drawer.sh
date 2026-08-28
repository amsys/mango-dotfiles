#!/bin/sh
# Toggles the tray drawer open/closed — genconfig.rs's tray_toggle_module()
# on_click_left. State lives in the `tray_open` ironvar, same bar-local
# reveal mechanism the tools drawer already uses
# (tools_module()'s own doc comment) — no daemon collector behind it.
set -u

if [ "${1:-}" = test ]; then
	echo ok
	exit 0
fi

case "$(ironbar var get tray_open 2>/dev/null)" in
true) ironbar var set tray_open false ;;
*) ironbar var set tray_open true ;;
esac
