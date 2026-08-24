#!/usr/bin/env bash
# Reports which packages this config needs and which are missing. Read-only —
# it never installs anything, because picking your AUR helper (and typing your
# sudo password into someone else's script) is your call, not this script's.
#
# Usage:
#   ./install-deps.sh            # report
#   ./install-deps.sh --dry-run  # identical; accepted so install.sh can pass it through
set -euo pipefail

log() { printf '%s\n' "$*"; }

# Package names as they appear on Arch. Several are AUR-only — see the split at
# the bottom, which decides per-package rather than hardcoding a list.
CORE_PKGS=(mangowm-git ironbar kitty rofi-wayland mako wlogout swaylock hypridle
	matugen swaybg cliphist wl-clipboard)
TOOL_PKGS=(grim slurp swappy hyprpicker tesseract tesseract-data-eng
	wf-recorder
	brightnessctl playerctl wireplumber networkmanager nm-connection-editor
	iw blueman pavucontrol-qt jq libnotify libpulse xdg-user-dirs
	atop btop dmidecode imagemagick python-gobject wayvnc kdeconnect)
LOOK_PKGS=(fish starship eza ttf-jetbrains-mono-nerd adw-gtk-theme-git
	breeze-plus kde-cli-tools ttf-ibm-plex ttf-material-symbols-variable-git)
# Referenced directly by fish/config.fish and git/config. Not cosmetic: without
# git-delta every paged git command fails outright, because git/config sets it
# as core.pager.
SHELL_PKGS=(fd fzf zoxide bat yazi git-delta)
OPTIONAL_PKGS=(keepassxc nextcloud-client dolphin arch-update)
# Nothing in this repo references these — they are here so the list of what
# makes this machine pleasant to use lives in one place rather than in memory.
SUGGESTED_PKGS=(tealdeer entr lazygit sd dust duf trash-cli satty udiskie
	wl-clip-persist)

log "-- checking dependencies --"

MISSING=()
for pkg in "${CORE_PKGS[@]}" "${TOOL_PKGS[@]}" "${LOOK_PKGS[@]}" "${SHELL_PKGS[@]}"; do
	pacman -Qq "$pkg" >/dev/null 2>&1 || MISSING+=("$pkg")
done
MISSING_OPT=()
for pkg in "${OPTIONAL_PKGS[@]}"; do
	pacman -Qq "$pkg" >/dev/null 2>&1 || MISSING_OPT+=("$pkg")
done
MISSING_SUGGESTED=()
for pkg in "${SUGGESTED_PKGS[@]}"; do
	pacman -Qq "$pkg" >/dev/null 2>&1 || MISSING_SUGGESTED+=("$pkg")
done

if ((${#MISSING[@]})); then
	log "  missing: ${MISSING[*]}"
	# pacman -S can't install AUR packages, so don't suggest a command that
	# would fail. Anything the sync db doesn't know is AUR.
	REPO_MISSING=()
	AUR_MISSING=()
	for pkg in "${MISSING[@]}"; do
		if pacman -Si "$pkg" >/dev/null 2>&1; then
			REPO_MISSING+=("$pkg")
		else
			AUR_MISSING+=("$pkg")
		fi
	done
	((${#REPO_MISSING[@]})) && log "  install: sudo pacman -S --needed ${REPO_MISSING[*]}"
	((${#AUR_MISSING[@]})) && log "  install (AUR): yay -S --needed ${AUR_MISSING[*]}"
else
	log "  all core/tool/look packages present"
fi

if ((${#MISSING_OPT[@]})); then
	log "  missing (optional, feature-gated): ${MISSING_OPT[*]}"
fi

if ((${#MISSING_SUGGESTED[@]})); then
	log "  missing (suggested, nothing depends on them): ${MISSING_SUGGESTED[*]}"
fi
