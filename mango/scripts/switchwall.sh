#!/usr/bin/env bash
# Wallpaper + colour-adaptation pipeline: sets the wallpaper, runs matugen over
# every template in matugen/config.toml, then nudges the apps that need telling.
#
# Ported from end-4/dots-hyprland (illogical-impulse), GPL-3.0 — see README
# Credits. Dropped its Hyprland/quickshell-only bits (video wallpaper, hyprctl
# monitor queries, AI categorization); added the swaybg + mmsg glue mango needs.
set -u

XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
XDG_STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THEME_FILE="$XDG_CONFIG_HOME/mango/theme.json"
STATE_DIR="$XDG_STATE_HOME/mango"

allowed_types=(scheme-content scheme-expressive scheme-fidelity scheme-fruit-salad scheme-monochrome scheme-neutral scheme-rainbow scheme-tonal-spot auto)

cfg() { [ -f "$THEME_FILE" ] && jq -r "$1" "$THEME_FILE" 2>/dev/null; }

set_wallpaper_path() {
	[ -f "$THEME_FILE" ] || return 0
	jq --arg path "$1" '.background.wallpaperPath = $path' "$THEME_FILE" >"$THEME_FILE.tmp" && mv "$THEME_FILE.tmp" "$THEME_FILE"
}

set_accent_color() {
	[ -f "$THEME_FILE" ] || return 0
	jq --arg color "$1" '.appearance.palette.accentColor = $color' "$THEME_FILE" >"$THEME_FILE.tmp" && mv "$THEME_FILE.tmp" "$THEME_FILE"
}

apply_wallpaper() {
	pkill -x swaybg 2>/dev/null || true
	setsid swaybg -i "$1" -m fill >/dev/null 2>&1 &
}

main() {
	local imgpath="" mode_flag="" type_flag="" color_flag="" color="" noswitch_flag=""

	while [[ $# -gt 0 ]]; do
		case "$1" in
		--mode) mode_flag="$2"; shift 2 ;;
		--type) type_flag="$2"; shift 2 ;;
		--color)
			if [[ "$2" =~ ^#?[A-Fa-f0-9]{6}$ ]]; then
				set_accent_color "$2"; shift 2
			elif [[ "$2" == "clear" ]]; then
				set_accent_color ""; shift 2
			else
				set_accent_color "$(hyprpicker --no-fancy)"; shift
			fi
			;;
		--image) imgpath="$2"; shift 2 ;;
		--noswitch)
			noswitch_flag="1"
			imgpath="$(cfg '.background.wallpaperPath')"
			shift
			;;
		*) [[ -z "$imgpath" ]] && imgpath="$1"; shift ;;
		esac
	done

	local config_color; config_color="$(cfg '.appearance.palette.accentColor')"
	if [[ "$config_color" =~ ^#?[A-Fa-f0-9]{6}$ ]]; then
		color_flag="1"; color="$config_color"
	fi

	[[ -z "$type_flag" ]] && type_flag="$(cfg '.appearance.palette.type')"
	[[ -z "$type_flag" || "$type_flag" == "null" ]] && type_flag="auto"
	# shellcheck disable=SC2076  # literal match, not regex
	if [[ ! " ${allowed_types[*]} " =~ " $type_flag " ]]; then
		echo "[switchwall] Warning: invalid type '$type_flag', defaulting to 'auto'" >&2
		type_flag="auto"
	fi

	if [[ -z "$imgpath" && -z "$color_flag" ]]; then
		cd "$(xdg-user-dir PICTURES)/Wallpapers/showcase" 2>/dev/null || cd "$(xdg-user-dir PICTURES)/Wallpapers" 2>/dev/null || cd "$(xdg-user-dir PICTURES)" || exit
		imgpath="$(kdialog --getopenfilename . --title 'Choose wallpaper')"
	fi

	if [[ -n "$imgpath" && -z "$noswitch_flag" ]]; then
		set_accent_color ""; color_flag=""; color=""
	fi

	[[ "$type_flag" == "auto" ]] && type_flag="scheme-tonal-spot"

	local matugen_args=(--source-color-index 0)
	if [[ "$color_flag" == "1" ]]; then
		matugen_args+=(color hex "$color")
	else
		[[ -z "$imgpath" ]] && { echo "Aborted"; exit 0; }
		matugen_args+=(image "$imgpath")
		set_wallpaper_path "$imgpath"
	fi

	# Apply the wallpaper now, before matugen/python — mango shows a black root color
	# until swaybg starts, so don't make the desktop wait on the whole color pipeline.
	[[ -n "$imgpath" ]] && apply_wallpaper "$imgpath"

	if [[ -z "$mode_flag" ]]; then
		mode_flag="dark"
		[[ "$(gsettings get org.gnome.desktop.interface color-scheme 2>/dev/null)" == *"prefer-light"* ]] && mode_flag="light"
	fi
	matugen_args+=(--mode "$mode_flag")
	matugen_args+=(--type "$type_flag")

	if [[ "$mode_flag" == "dark" ]]; then
		gsettings set org.gnome.desktop.interface color-scheme 'prefer-dark' 2>/dev/null || true
	else
		gsettings set org.gnome.desktop.interface color-scheme 'prefer-light' 2>/dev/null || true
	fi
	mkdir -p "$STATE_DIR/generated"

	if [[ "$(cfg '.appearance.wallpaperTheming.enableAppsAndShell')" == "false" ]]; then
		echo "App/shell theming disabled, skipping matugen"
		return
	fi

	# Regenerates every matugen template in ~/.config/matugen/config.toml, including
	# the mango colors.conf and kitty theme.conf targets.
	matugen "${matugen_args[@]}"
	pkill -SIGUSR2 waybar 2>/dev/null || true

	# Qt/KDE + GTK3: matugen just wrote the palette into kdeglobals/gtk.css, but
	# icon theme and gtk-theme are name-switched, not color-switched, so they
	# live here. --notify broadcasts the KConfig change so running KDE apps
	# repaint without a restart.
	if [[ "$mode_flag" == "dark" ]]; then
		kwriteconfig6 --file kdeglobals --group Icons --key Theme breeze-plus-dark 2>/dev/null || true
		gsettings set org.gnome.desktop.interface gtk-theme 'adw-gtk3-dark' 2>/dev/null || true
	else
		kwriteconfig6 --file kdeglobals --group Icons --key Theme breeze-plus 2>/dev/null || true
		gsettings set org.gnome.desktop.interface gtk-theme 'adw-gtk3' 2>/dev/null || true
	fi
	kwriteconfig6 --file kdeglobals --group General --key ColorScheme MaterialYou --notify 2>/dev/null || true

	"$SCRIPT_DIR/vscode-set-color.sh" &

	mmsg dispatch reload_config >/dev/null 2>&1 || true
	"$SCRIPT_DIR/keybinds-cheatsheet.py" >/dev/null 2>&1 || true
}

main "$@"
