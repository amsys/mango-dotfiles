#!/usr/bin/env bash
# Symlinks this repo into ~/.config, one file at a time — never a directory
# symlink, so matugen's generated output lands as a plain file next to the
# symlinks and never dirties this repo.
#
# Usage:
#   ./install-config.sh            # install
#   ./install-config.sh --dry-run  # print every action, change nothing
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
XDG_STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
DRY_RUN=0
[[ "${1:-}" == "--dry-run" ]] && DRY_RUN=1

STAMP=$(date +%Y%m%d-%H%M%S)
LINKED=0
BACKED_UP=0

log() { printf '%s\n' "$*"; }

# link SRC DEST — backs up an existing regular file/dir, symlinks otherwise.
# No-op if DEST is already the correct symlink.
link() {
	local src="$1" dest="$2"
	if [[ -L "$dest" && "$(readlink -f "$dest")" == "$(readlink -f "$src")" ]]; then
		return 0
	fi
	if [[ -e "$dest" || -L "$dest" ]]; then
		log "  backup: $dest -> $dest.bak.$STAMP"
		((DRY_RUN)) || mv "$dest" "$dest.bak.$STAMP"
		BACKED_UP=$((BACKED_UP + 1))
	fi
	log "  link:   $dest -> $src"
	if ((DRY_RUN)); then
		:
	else
		mkdir -p "$(dirname "$dest")"
		ln -sfn "$src" "$dest"
	fi
	LINKED=$((LINKED + 1))
}

# link_tree SRC_DIR DEST_DIR — per-file symlink for every regular file under
# SRC_DIR, skipping *.example files.
link_tree() {
	local src_dir="$1" dest_dir="$2"
	[[ -d "$src_dir" ]] || return 0
	while IFS= read -r -d '' file; do
		local rel="${file#"$src_dir"/}"
		[[ "$rel" == *.example ]] && continue
		link "$file" "$dest_dir/$rel"
	done < <(find "$src_dir" -type f -print0)
}

# copy_if_absent SRC DEST — used for runtime-mutable / per-machine files that
# must never be overwritten once they exist.
copy_if_absent() {
	local src="$1" dest="$2"
	if [[ -e "$dest" ]]; then
		log "  keep:   $dest (already exists, not touched)"
		return 0
	fi
	log "  create: $dest (from $(basename "$src"))"
	((DRY_RUN)) || { mkdir -p "$(dirname "$dest")"; cp "$src" "$dest"; }
}

((DRY_RUN)) && log "(dry run — no changes will be made)"

log "-- symlinking config --"
for dir in mango waybar kitty rofi matugen wlogout fish fontconfig git; do
	log "$dir/"
	link_tree "$REPO/$dir" "$XDG_CONFIG_HOME/$dir"
done
log "starship.toml"
link "$REPO/starship.toml" "$XDG_CONFIG_HOME/starship.toml"
log

log "-- per-machine files (created once, never overwritten) --"
copy_if_absent "$REPO/mango/theme.json.example" "$XDG_CONFIG_HOME/mango/theme.json"
copy_if_absent "$REPO/mango/local.conf.example" "$XDG_CONFIG_HOME/mango/local.conf"
copy_if_absent "$REPO/git/config.local.example" "$XDG_CONFIG_HOME/git/config.local"
log
log "  Both need editing before they do anything:"
log "    git/config.local  — your name and email, or git refuses to commit"
log "    mango/local.conf  — XDG_DATA_DIRS needs your absolute home path"
log "                        (mango does not expand \$HOME in env= values)"
log

# /sys/firmware/dmi is 0400, so the memory tooltip cannot read DIMM details as
# your user. Cache them once here instead of adding a sudoers rule; the tooltip
# just drops its Hardware section if this never runs.
DMI_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/mango-meminfo"
log "-- DIMM details for the memory tooltip --"
if [[ -s "$DMI_CACHE" ]]; then
	log "  keep:   $DMI_CACHE (already cached)"
elif ((DRY_RUN)); then
	log "  (dry run) would run: sudo dmidecode -t memory > $DMI_CACHE"
elif ! command -v dmidecode >/dev/null 2>&1; then
	log "  skip:   dmidecode not installed"
elif sudo -n true 2>/dev/null || sudo -v 2>/dev/null; then
	mkdir -p "$(dirname "$DMI_CACHE")"
	if sudo dmidecode -t memory >"$DMI_CACHE" 2>/dev/null; then
		log "  create: $DMI_CACHE"
	else
		rm -f "$DMI_CACHE"
		log "  skip:   dmidecode failed"
	fi
else
	log "  skip:   no sudo available (rerun interactively to populate it)"
fi
log

# The Nextcloud client publishes bare SNI icon names — state-ok, state-sync,
# state-error, state-warning, state-offline, state-pause — with an empty
# IconThemePath *and* an empty IconPixmap, so the tray host has nothing to fall
# back on but the name. Those names ship only in breeze/breeze-icons; under
# Adwaita waybar cannot resolve them and draws a generic placeholder instead.
# Alias them onto the branded icons nextcloud-client already installs. hicolor
# is the target because every GTK icon theme falls back to it, so this works
# whatever icon theme is set later. offline/pause have no branded artwork, so
# they land on the plain cloud alongside ok.
NC_ICONS=/usr/share/icons/hicolor
ICON_DEST="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
log "-- Nextcloud tray icon names --"
if [[ -f "$NC_ICONS/64x64/apps/Nextcloud.png" ]]; then
	for map in state-ok:Nextcloud state-offline:Nextcloud state-pause:Nextcloud \
		state-sync:Nextcloud_sync state-error:Nextcloud_error \
		state-warning:Nextcloud_warn; do
		for size in 16 24 32 48 64; do
			src="$NC_ICONS/${size}x${size}/apps/${map##*:}.png"
			[[ -f "$src" ]] || continue
			link "$src" "$ICON_DEST/${size}x${size}/status/${map%%:*}.png"
		done
	done
else
	log "  skip:   nextcloud-client icons not installed"
fi
log

log "-- materializing matugen output --"
if ((DRY_RUN)); then
	log "  (dry run) would run: mango/scripts/switchwall.sh --noswitch"
elif ! command -v matugen >/dev/null 2>&1; then
	log "  WARNING: matugen not installed — skipping. Generated files (waybar"
	log "  style, kitty theme, swaylock config, rofi colors, ...) won't exist"
	log "  until you install matugen and run switchwall.sh yourself."
else
	mkdir -p "$XDG_STATE_HOME/mango/generated/wallpaper"
	if "$XDG_CONFIG_HOME/mango/scripts/switchwall.sh" --noswitch; then
		log "  ok"
	else
		log "  WARNING: switchwall.sh --noswitch failed — no wallpaper is set yet."
		log "  Run 'SUPER+W' or './mango/scripts/switchwall.sh <image>' once logged in."
	fi
fi
log

log "== $LINKED linked, $BACKED_UP backed up =="
