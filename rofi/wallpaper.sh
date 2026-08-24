#!/usr/bin/env bash
# rofi script mode: pick a wallpaper by thumbnail grid, replacing kdialog's
# plain (preview-less) file dialog — kdialog has no preview flag at all, and
# the KDE dialog's own persisted preview state (~/.config/kdialogrc) doesn't
# exist on a fresh machine. This uses the same icon-row mechanism
# clipboard.sh already proves works on this rofi build: an absolute image
# path in the `icon` row option renders as a real thumbnail, no icon theme
# involved.
#
#   wallpaper.sh --launch   spawn rofi with this mode, print the picked
#                           absolute path on stdout (nothing if cancelled)
#   wallpaper.sh            rofi script-mode callback
#   wallpaper.sh test       assert the row builder against a temp directory
#
# rofi calls the script fresh for the ROFI_RETV=1 selection callback — a
# separate process from the --launch invocation, which has no way to read
# that process's stdout (unlike window.sh's `mmsg dispatch`, a wallpaper
# pick has no daemon to hand the result to). WALLPAPER_PICK_OUT, set by
# --launch before spawning rofi and inherited by every script-mode child, is
# the hand-off: the callback writes the chosen path there, --launch reads it
# back once rofi exits.
set -euo pipefail

THEME="$HOME/.config/rofi/wallpaper.rasi"

opt() { printf '\0%s\x1f%s\n' "$1" "$2"; }

# First existing directory wins: the real wallpaper location, then the two
# switchwall.sh's own kdialog call used to fall through to (xdg-user-dir has
# no PICTURES/Wallpapers entry on this machine, so those two often collapse
# to the same directory anyway).
wallpaper_dir() {
	local d
	for d in "$HOME/Wallpapers" "$(xdg-user-dir PICTURES)/Wallpapers" "$(xdg-user-dir PICTURES)"; do
		[[ -d "$d" ]] && { printf '%s' "$d"; return 0; }
	done
	return 1
}

# One rofi row per image file directly under $1 (no recursion — kdialog's
# own picker didn't recurse either): "<filename>\0icon\x1f<path>\x1finfo\x1f<path>".
# The source image itself is the icon, rofi scales it — no thumbnail cache
# for the handful of wallpapers this ever lists (see the plan's own note on
# when that stops being true).
rows() {
	local dir="$1" f name
	while IFS= read -r -d '' f; do
		[[ "${f,,}" =~ \.(jpg|jpeg|png|webp)$ ]] || continue
		name="$(basename "$f")"
		printf '%s\0icon\x1f%s\x1finfo\x1f%s\n' "$name" "$f" "$f"
	done < <(find "$dir" -maxdepth 1 -type f -print0 | sort -z)
}

# ---------------------------------------------------------------- selftest

if [[ ${1:-} == test ]]; then
	tmp="$(mktemp -d)"
	trap 'rm -rf "$tmp"' EXIT
	touch "$tmp/b.jpg" "$tmp/a.png" "$tmp/notes.txt" "$tmp/c.JPEG"
	out="$(rows "$tmp" | tr '\000' '@')"

	[[ $(wc -l <<< "$out") == 3 ]] || { echo "expected 3 image rows, got: $out"; exit 1; }
	[[ "$out" != *"notes.txt"* ]] || { echo "non-image file leaked into the rows"; exit 1; }
	[[ $(sed -n 1p <<< "$out") == a.png* ]] || { echo "rows not sorted: $(sed -n 1p <<< "$out")"; exit 1; }
	[[ $(sed -n 3p <<< "$out") == *"c.JPEG"* ]] || { echo "uppercase extension not matched"; exit 1; }
	[[ $(sed -n 1p <<< "$out") == "a.png@icon"$'\x1f'"$tmp/a.png"$'\x1f'"info"$'\x1f'"$tmp/a.png" ]] \
		|| { echo "icon/info must both be the absolute path: $(sed -n 1p <<< "$out")"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- launcher

if [[ ${1:-} == --launch ]]; then
	out="$(mktemp)"
	trap 'rm -f "$out"' EXIT
	WALLPAPER_PICK_OUT="$out" rofi -show wallpaper -modes "wallpaper:$HOME/.config/rofi/wallpaper.sh" -theme "$THEME"
	[[ -s "$out" ]] && cat "$out"
	exit 0
fi

# ---------------------------------------------------------------- mode

if [[ ${ROFI_RETV:-0} == 1 ]]; then
	# ROFI_INFO is only promised on a selection, which is all we have.
	[[ -n ${ROFI_INFO:-} && -n ${WALLPAPER_PICK_OUT:-} ]] && printf '%s' "$ROFI_INFO" > "$WALLPAPER_PICK_OUT"
	exit 0
fi

dir="$(wallpaper_dir)" || exit 0
opt no-custom true
opt message "$dir"
rows "$dir"
