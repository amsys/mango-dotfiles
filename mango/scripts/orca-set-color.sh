#!/usr/bin/env bash
# Applies the matugen palette to Orca (stably-orca-bin), the Electron IDE.
#
# Orca has no theme file. Its settings live under the "settings" key of a large
# app-state blob, orca-data.json, which also holds repos, projects, worktrees
# and sessions. matugen therefore cannot own that file: it renders both terminal
# themes into the state dir, and this script patches them in — the same split
# vscode-set-color.sh uses for the VS Code forks.
#
# jq + mktemp-in-target-dir + mv, never sed: jq sets or replaces a key with one
# expression, and a jq parse failure writes nothing instead of a corrupted file
# (the M8 lesson from vscode-set-color.sh). mktemp sits in the target's own
# directory so mv stays on one filesystem and is atomic.
#
# Orca reads orca-data.json once at startup and never watches it. It writes the
# whole blob back from its in-memory state (buildStateToSave), and it does that
# on a 1-5 s debounce after ANY state change — a tab switch or a window resize
# is enough. An on-disk edit never enters that in-memory state, so patching a
# running Orca is undone the moment the user touches it.
#
# So this script only patches while Orca is stopped. That is not a limitation in
# practice: config.conf runs switchwall.sh --noswitch at login, before Orca
# starts, so a wallpaper picked in the last session is applied there. Skipping a
# running Orca also removes any chance of this script clobbering the repos,
# worktrees and sessions that share the file.
#
# Exits 0 on every path. A wallpaper switch must not fail because of Orca.
set -u

THEMES="${XDG_STATE_HOME:-$HOME/.local/state}/mango/generated/orca-themes.json"
ORCA_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/orca"
INDEX="$ORCA_DIR/orca-profile-index.json"

[[ -f "$THEMES" ]] || { echo "orca-set-color: $THEMES missing, nothing to set" >&2; exit 0; }
[[ -d "$ORCA_DIR" ]] || exit 0

# Orca rewrites this file from memory, so a patch applied now would be lost.
# Leave it alone; the render stays in the state dir and lands at next login.
if pgrep -x orca-ide >/dev/null 2>&1; then
	echo "orca-set-color: Orca is running, deferring to next start" >&2
	exit 0
fi

# The profile directory name is not fixed; orca-profile-index.json names the
# active one. Fall back to the id Orca creates on first run.
profile=$(jq -r '.activeProfileId // "local-default"' "$INDEX" 2>/dev/null)
[[ -n "$profile" && "$profile" != "null" ]] || profile="local-default"

DATA="$ORCA_DIR/profiles/$profile/orca-data.json"
[[ -f "$DATA" ]] || exit 0

tmp=$(mktemp "$DATA.XXXXXX") || exit 0
if ! jq --slurpfile t "$THEMES" '
      ($t[0]) as $th
    | .settings.terminalCustomThemes =
        (((.settings.terminalCustomThemes // [])
          | map(select(.id != $th.dark.id and .id != $th.light.id)))
         + [$th.dark, $th.light])
    | .settings.terminalThemeDark  = ("custom:" + $th.dark.id)
    | .settings.terminalThemeLight = ("custom:" + $th.light.id)
    | .settings.terminalUseSeparateLightTheme = true
    | .settings.terminalDividerColorDark  = $th.dividerDark
    | .settings.terminalDividerColorLight = $th.dividerLight
    | .settings.leftSidebarAppearanceMode = "match-terminal"
   ' "$DATA" >"$tmp"; then
	echo "orca-set-color: skipping $DATA — jq could not parse it" >&2
	rm -f "$tmp"
	exit 0
fi

# Only write when the palette actually changed. Orca rewrites this file from
# memory on its own schedule, so every needless write is another chance to lose
# an update it made between our read and our mv.
if cmp -s "$tmp" "$DATA"; then
	rm -f "$tmp"
	exit 0
fi

cp -p "$DATA" "$DATA.mango.bak" 2>/dev/null || true
mv "$tmp" "$DATA"
