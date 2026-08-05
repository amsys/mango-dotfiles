#!/usr/bin/env bash
# Full install: check dependencies, then symlink the config into ~/.config.
# Both halves are runnable on their own — ./install-deps.sh reports packages
# without touching anything, ./install-config.sh links without checking them.
#
# Usage:
#   ./install.sh            # install
#   ./install.sh --dry-run  # print every action, change nothing
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
ARG="${1:-}"

log() { printf '%s\n' "$*"; }

log "== mango-dotfiles install =="
log "repo:   $REPO"
log "target: $XDG_CONFIG_HOME"
log

"$REPO/install-deps.sh" "$ARG"
log

"$REPO/install-config.sh" "$ARG"
log

log "Manual steps still needed:"
log "  - Install Google Sans Flex if flagged above"
log "  - KeePassXC: the entry holding the OpenRouter key must carry the attribute"
log "    application=mango (was: illogical-impulse). Edit it under Advanced ->"
log "    Additional attributes, or rofi's Alt+I returns 'not found'."
log "  - fish/conf.d/claude.fish is NOT installed (contained a live API key on"
log "    the source machine) — recreate it from the keyring, not from history"
log "  - git: delete ~/.gitconfig once git-delta is installed. It shadows the"
log "    tracked ~/.config/git/config (git reads XDG first, \$HOME second, later"
log "    wins), so until it is gone none of the delta settings apply."
log "  - Review mango/local.conf and mango/theme.json for values specific to the"
log "    OLD machine (monitor name, XDG_DATA_DIRS, wallpaper path)"
log "  - Login screen: sudo system/sddm/install.sh  (installs the mango-sddm theme,"
log "    the root-owned colour sync tool and its sudoers rule; nothing under"
log "    system/ is symlinked, so this is the only way it lands)"
log "  - See README.md's checklist for what this repo intentionally does not cover"
log "    (hypridle, KeePassXC autounlock chain, mimeapps.list, SDDM theme, ...)"
