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
	matugen awww cliphist wl-clipboard
	xdg-desktop-portal xdg-desktop-portal-gtk xdg-desktop-portal-wlr)
TOOL_PKGS=(grim slurp swappy hyprpicker tesseract tesseract-data-eng
	wf-recorder wlopm
	brightnessctl playerctl wireplumber networkmanager nm-connection-editor
	iw blueman pavucontrol-qt jq libnotify libpulse xdg-user-dirs
	btop dmidecode imagemagick python-gobject wayvnc kdeconnect upower
	qt6-wayland)
LOOK_PKGS=(fish starship eza ttf-jetbrains-mono-nerd adw-gtk-theme-git
	breeze-plus kde-cli-tools ttf-ibm-plex ttf-material-symbols-variable-git
	darkly-bin ttf-rubik-vf)
# Referenced directly by fish/config.fish and git/config. Not cosmetic: without
# git-delta every paged git command fails outright, because git/config sets it
# as core.pager.
SHELL_PKGS=(fd fzf zoxide bat yazi git-delta wget)
# qt5-wayland pairs with keepassxc and qt6-wayland (TOOL_PKGS) with
# pavucontrol-qt: Qt does not depend on its own Wayland plugin, so without it
# a Qt app does not fail, it silently starts on XWayland under a different
# appid and every windowrule that names its Wayland app_id stops matching.
OPTIONAL_PKGS=(keepassxc qt5-wayland nextcloud-client dolphin arch-update)
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

# A package installed --asdeps lives only as long as whatever pulled it in.
# When that holder goes, `pacman -Rns $(pacman -Qtdq)` takes this one too —
# that is how qt5-wayland vanished on 2026-09-01 and put KeePassXC on XWayland.
# `pacman -Qi` (not -Qeq) so a provides-name like rofi-wayland still resolves
# to its real package (rofi) and reads that package's own install reason.
ASDEPS=()
for pkg in "${CORE_PKGS[@]}" "${TOOL_PKGS[@]}" "${LOOK_PKGS[@]}" \
	"${SHELL_PKGS[@]}" "${OPTIONAL_PKGS[@]}"; do
	reason=$(pacman -Qi "$pkg" 2>/dev/null | awk -F': ' '/^Install Reason/{print $2}')
	[[ -z $reason || $reason == Explicit* ]] || ASDEPS+=("$pkg")
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

if ((${#ASDEPS[@]})); then
	log "  installed as a dependency, an orphan sweep can remove them: ${ASDEPS[*]}"
	log "  pin: sudo pacman -D --asexplicit ${ASDEPS[*]}"
fi
