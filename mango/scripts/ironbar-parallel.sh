#!/usr/bin/env bash
# Runs mango-bard + ironbar alongside the still-live waybar, for testing
# IRONBAR.md's staged tasks on a real session. Temporary: T8 replaces this
# with the real cutover (systemd unit, matugen theming, install-config.sh
# symlink wiring for src/ironbar/). Until then this is the only launch path,
# and it never touches waybar/bars.sh — both bars run side by side, exactly
# as T0 spike S5 confirmed is safe (opposite exclusive zones, no conflict).
#
# ironbar defaults position=bottom with no config here saying otherwise —
# "as now" alongside waybar's own top bar. A top-positioned bar is future
# work (see IRONBAR.md); ironbar computes popup/tooltip opening direction
# from the bar's own `position` itself (confirmed via `ironbar
# --print-schema`: there is no separate popup-direction option to set), so
# switching `position` later needs no extra plumbing here.
#
#   ironbar-parallel.sh    exec-once from mango/config.conf
set -euo pipefail

# $0 is the ~/.config symlink install-config.sh's link_tree creates
# (mango/ is symlinked file-by-file, not as one directory), so readlink -f
# is needed here — unlike every other mango/scripts/ script, this one has
# to reach across into src/ironbar/, a sibling top-level dir that install-
# config.sh's own symlink loop does not cover yet.
REPO="$(cd "$(dirname "$(readlink -f "$0")")/../.." && pwd)"
BARD_DIR="$REPO/ironbar/bard"
BARD_BIN="$BARD_DIR/target/release/mango-bard"
CONFIG="${XDG_RUNTIME_DIR:-/tmp}/ironbar-parallel-config.json"

# Build once, or whenever a source file changed since — same staleness
# check install-config.sh's fast-tooltips.c shim uses for the same reason
# (skip the ~1-2s rebuild on every login when nothing changed).
if [ ! -x "$BARD_BIN" ] || [ -n "$(find "$BARD_DIR/src" -name '*.rs' -newer "$BARD_BIN")" ]; then
	( cd "$BARD_DIR" && cargo build --release )
fi

# T6b: hotspot.sh's toggle() and switchwall.sh both call `mango-bard refresh
# <topic>` directly (not through this script), so the binary needs to be on
# PATH — $HOME/.local/bin is first on PATH already (fish/config.fish),
# unlike $BARD_BIN's build-output path.
mkdir -p "$HOME/.local/bin"
ln -sf "$BARD_BIN" "$HOME/.local/bin/mango-bard"

"$BARD_BIN" gen-config --out "$CONFIG"
"$BARD_BIN" run &
exec ironbar -c "$CONFIG" -t "$REPO/ironbar/style.css"
