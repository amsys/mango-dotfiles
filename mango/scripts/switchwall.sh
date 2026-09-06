#!/usr/bin/env bash
# Wallpaper + colour-adaptation pipeline: sets the wallpaper, runs matugen over
# every template in matugen/config.toml, then nudges the apps that need telling.
#
# Ported from end-4/dots-hyprland (illogical-impulse), GPL-3.0 — see README
# Credits. Dropped its Hyprland/quickshell-only bits (video wallpaper, hyprctl
# monitor queries, AI categorization); added the awww + mmsg glue mango needs.
set -u

XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
XDG_STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
THEME_FILE="$XDG_CONFIG_HOME/mango/theme.json"
STATE_DIR="$XDG_STATE_HOME/mango"

allowed_types=(scheme-content scheme-expressive scheme-fidelity scheme-fruit-salad scheme-monochrome scheme-neutral scheme-rainbow scheme-tonal-spot auto)

cfg() { [ -f "$THEME_FILE" ] && jq -r "$1" "$THEME_FILE" 2>/dev/null; }

# mktemp inside $THEME_FILE's own directory, not a fixed ".tmp" name: two
# concurrent writers sharing one fixed temp path can interleave their
# truncate/write/mv and install half-written JSON, which every later cfg()
# read then silently returns nothing for. Same directory keeps `mv` on one
# filesystem, so it stays atomic.
theme_write() { # jq-filter jq-arg...
	local filter="$1" tmp
	shift
	tmp=$(mktemp "$THEME_FILE.XXXXXX") || return 1
	jq "$@" "$filter" "$THEME_FILE" >"$tmp" && mv "$tmp" "$THEME_FILE" || rm -f "$tmp"
}

set_wallpaper_path() {
	[ -f "$THEME_FILE" ] || return 0
	theme_write '.background.wallpaperPath = $path' --arg path "$1"
}

set_accent_color() {
	[ -f "$THEME_FILE" ] || return 0
	theme_write '.appearance.palette.accentColor = $color' --arg color "$1"
}

SPAN_DIR="$STATE_DIR/generated/wallpaper"

# T-span: an ultrawide panorama (e.g. 3840x1080 across two 1920x1080 outputs)
# cover-crops identically on every output with one awww image (no -o),
# showing the same ~2x-zoomed centre slice twice instead of spanning.
# span_geometry prints per-output crop rects when the image aspect matches
# the monitor layout's bounding-box aspect, or REJECT otherwise (fewer than
# 2 outputs, or the aspect is off by more than 10% — the real
# 3840x1080/5120x1440 panoramas score 0.00, a 3440x1440 ultrawide on this
# layout scores 0.33, a near-square wallpaper scores 0.50, so 0.10 cleanly
# separates "is a panorama for this layout" from "isn't"). Crop is computed
# in logical (layout) coordinates then scaled into source pixels — awww
# rescales the tile to the output's physical mode regardless of its own
# resolution, so cropping from the source instead of downscaling first is
# what keeps a higher-than-layout-resolution panorama (e.g. 5120x1440) sharp.
span_geometry() {
	local sw="$1" sh="$2" mons
	mons="$(mmsg get all-monitors 2>/dev/null)" || return 1
	jq -n -r --argjson mons "$mons" --argjson sw "$sw" --argjson sh "$sh" '
		($mons.monitors // []) as $all
		| ($all | map(select(.width > 0 and .height > 0))) as $m
		| if ($m | length) < 2 then "REJECT" else
			($m | map(.x) | min) as $minx |
			($m | map(.y) | min) as $miny |
			($m | map(.x + .width) | max) as $maxx |
			($m | map(.y + .height) | max) as $maxy |
			($maxx - $minx) as $boxw |
			($maxy - $miny) as $boxh |
			([$sw / $boxw, $sh / $boxh] | min) as $k |
			(($sw / $sh) / ($boxw / $boxh)) as $ratio |
			((if $ratio < 1 then 1 / $ratio else $ratio end) - 1) as $diff |
			if $diff > 0.10 then "REJECT" else
				(($sw - $k * $boxw) / 2) as $offx |
				(($sh - $k * $boxh) / 2) as $offy |
				(["ACCEPT"] + ($m | map(
					"\(.name) \((.width * $k) | round) \((.height * $k) | round) \(($offx + (.x - $minx) * $k) | round) \(($offy + (.y - $miny) * $k) | round)"
				))) | join("\n")
			end
		end
	'
}

# Cuts (or reuses cached) per-output tiles for image "$1", printing
# "name tilepath" lines on success. Returns 1 with no stderr — this is the
# ordinary "not an ultrawide for this layout" path, not an error condition —
# when the gate fails or ImageMagick isn't installed; apply_wallpaper()
# falls back to today's single-image fill either way. JPEG tiles, not PNG:
# measured 0.09s vs 0.90s per tile on this machine, and this path runs
# before the desktop shows anything but black.
span_tiles() {
	command -v magick >/dev/null 2>&1 || return 1
	local img="$1" sw sh geometry key name w h x y
	# `read`'s own exit status is unreliable here: identify's format string
	# has no trailing newline, so read hits EOF right after the last field
	# and reports failure even though sw/sh parsed fine — check the values
	# themselves instead of the read command's status.
	read -r sw sh < <(identify -format '%w %h' "$img" 2>/dev/null)
	[[ -n "${sw:-}" && -n "${sh:-}" ]] || return 1
	geometry="$(span_geometry "$sw" "$sh")" || return 1
	[[ "$geometry" == ACCEPT* ]] || return 1

	mkdir -p "$SPAN_DIR"
	key="$img $(stat -c %Y "$img" 2>/dev/null)
$geometry"
	if [[ -f "$SPAN_DIR/span.key" && "$(cat "$SPAN_DIR/span.key")" == "$key" ]]; then
		tail -n +2 <<<"$geometry" | while read -r name _; do
			echo "$name $SPAN_DIR/span-$name.jpg"
		done
		return 0
	fi

	rm -f "$SPAN_DIR"/span-*.jpg
	while read -r name w h x y; do
		magick "$img" -crop "${w}x${h}+${x}+${y}" +repage "$SPAN_DIR/span-$name.jpg" || return 1
		echo "$name $SPAN_DIR/span-$name.jpg"
	done < <(tail -n +2 <<<"$geometry")
	printf '%s' "$key" >"$SPAN_DIR/span.key"
}

# Nothing re-runs this script on monitor hotplug today (one exec-once at
# login, one SUPER+W bind) — a newly plugged monitor needs SUPER+W pressed
# once to pick up its own tile, same as ironbar already needs for its own
# per-monitor bars. Known limitation, not a watcher: YAGNI until it bites.
# awww swaps the image inside one long-lived daemon. No layer surface is
# dropped, so a switch never shows the root color, and the change can fade.
# config.conf puts the blurred boot image up before this script runs, so the
# first call of the session dissolves that image into the sharp wallpaper.
apply_wallpaper() {
	local tiles name path
	local fade=(--resize crop --transition-type fade
		--transition-duration 1 --transition-fps 60)
	# The daemon is normally already up from config.conf. Start it here too:
	# SUPER+W must still work if it died, and this script runs standalone.
	if ! awww query >/dev/null 2>&1; then
		setsid awww-daemon --no-cache --quiet >/dev/null 2>&1 &
		for _ in $(seq 60); do
			awww query >/dev/null 2>&1 && break
			sleep 0.05
		done
	fi
	if tiles="$(span_tiles "$1")" && [[ -n "$tiles" ]]; then
		while read -r name path; do
			awww img -o "$name" "$path" "${fade[@]}" >/dev/null 2>&1
		done <<<"$tiles"
		return 0
	fi
	awww img "$1" "${fade[@]}" >/dev/null 2>&1
}

main() {
	local imgpath="" mode_flag="" type_flag="" color_flag="" color="" noswitch_flag=""

	while [[ $# -gt 0 ]]; do
		case "$1" in
		--mode) mode_flag="$2"; shift 2 ;;
		--type) type_flag="$2"; shift 2 ;;
		--color)
			if [[ "${2:-}" =~ ^#?[A-Fa-f0-9]{6}$ ]]; then
				set_accent_color "$2"; shift 2
			elif [[ "${2:-}" == "clear" ]]; then
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

	# rofi thumbnail grid, not kdialog: kdialog has no preview flag at all,
	# and the KDE dialog's own persisted preview state (~/.config/kdialogrc)
	# doesn't exist on a fresh machine, so it always opened as a bare list.
	if [[ -z "$imgpath" && -z "$color_flag" ]]; then
		imgpath="$("$XDG_CONFIG_HOME/rofi/wallpaper.sh" --launch)"
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
	# until awww paints, so don't make the desktop wait on the whole color pipeline.
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
	# No reload poke for ironbar itself — it hot-loads its CSS file on
	# change (confirmed live, IRONBAR.md T8a), unlike waybar, which needed
	# the SIGUSR2 this used to also send. darkmode.rs has no watcher of its
	# own though (IRONBAR.md T6b decision D3 — a gsettings monitor child
	# measured ~22 ctxt-switches/min idle, worse than every other watcher
	# this daemon runs), so its ironvar still needs this explicit poke.
	# "colors" (not "darkmode"): also re-reads generated/colors.json into
	# mango-bard's popup palette (tooltip.rs's reload_palette()) before
	# doing the same full resync "darkmode" used to trigger alone.
	mango-bard refresh colors 2>/dev/null || true

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
	"$SCRIPT_DIR/orca-set-color.sh" &

	mmsg dispatch reload_config >/dev/null 2>&1 || true
	"$SCRIPT_DIR/keybinds-cheatsheet.py" >/dev/null 2>&1 || true
}

main "$@"
