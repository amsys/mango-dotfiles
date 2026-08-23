#!/usr/bin/env bash
# Bar launcher — exec-once'd from mango/config.conf. Replaces waybar/bars.sh
# at the T8 cutover (IRONBAR.md). mango-bard itself runs as its own systemd
# user unit (mango-bard.service), started independently — this script only
# generates the bar config from the live monitor list and execs ironbar.
set -euo pipefail

CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/ironbar/config.json"
mango-bard gen-config --out "$CONFIG"

# GTK3's hardcoded 500ms hover-tooltip delay (see install-config.sh's own
# comment on the shim) — carried over from bars.sh's `env LD_PRELOAD=...
# waybar`. Skipped, not fatal, if install-config.sh couldn't build it (no
# cc/glib-2.0 dev headers): the bar still runs, just with the stock delay.
SHIM="$HOME/.local/lib/mango/fast-tooltips.so"
[ -f "$SHIM" ] && export LD_PRELOAD="$SHIM"

exec ironbar
