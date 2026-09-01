#!/usr/bin/env bash
# Keep something other than the firmware logo on screen through GRUB. Safe to
# re-run.
#
#   sudo ~/src/mango-dotfiles/system/grub/install.sh          # variant 1
#   sudo ~/src/mango-dotfiles/system/grub/install.sh --gfx    # variant 2
#
# Variant 1 (default): hidden menu, console output. GRUB draws nothing, so the
# framebuffer the firmware left (the ASUS logo) survives until plymouth.
#
# Variant 2 (--gfx): gfxterm with the blurred wallpaper as the background, the
# same image plymouth and SDDM already show
# (/boot/grub/mango-bg.png, kept current by
# system/plymouth/plymouth-theme-sync on every wallpaper switch). Use it when
# variant 1 clears the screen, or when the firmware's EFI text console shows
# nothing at all — on the ASUS B1502 the variant 1 menu is a blank screen, so
# the rescue entries cannot be seen. It also closes the gap where the ASUS
# logo used to reappear between the LUKS prompt and the SDDM greeter: that
# gap shows the framebuffer GRUB itself last drew, so drawing the wallpaper
# here carries it through the gap too.
#
# GRUB opens the hidden menu on Esc, F4 or a held Shift during the 2 s
# timeout (grub_key_is_interrupt); this firmware also reacts to other keys.
#
# Both variants keep every generated entry (Windows, the pinned one). The
# "Loading ..." echo lines are dropped by grub-regen for the same reason.

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULTS=/etc/default/grub
GRUB_BG=/boot/grub/mango-bg.png
STAMP="$(date +%Y%m%d-%H%M%S)"
variant=console

case "${1:-}" in
	'') ;;
	--gfx) variant=gfx ;;
	*) echo "usage: $0 [--gfx]" >&2; exit 1 ;;
esac

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
for cmd in grub-mkconfig sed; do
	command -v "$cmd" >/dev/null || { echo "missing dependency: $cmd" >&2; exit 1; }
done

# set_kv KEY VALUE — replace an active line, or append one.
set_kv() {
	if grep -q "^$1=" "$DEFAULTS"; then
		sed -i "s|^$1=.*|$1=$2|" "$DEFAULTS"
	else
		printf '\n%s=%s\n' "$1" "$2" >>"$DEFAULTS"
	fi
}

echo "==> grub-regen -> /usr/local/bin/grub-regen"
install -m 0755 -o root -g root "$SRC_DIR/grub-regen" /usr/local/bin/grub-regen

echo "==> $DEFAULTS (backup: $DEFAULTS.bak.$STAMP)"
cp -a "$DEFAULTS" "$DEFAULTS.bak.$STAMP"
set_kv GRUB_TIMEOUT_STYLE hidden
set_kv GRUB_TIMEOUT 2

if [ "$variant" = console ]; then
	set_kv GRUB_TERMINAL_OUTPUT console
	sed -i '/^GRUB_BACKGROUND=/d' "$DEFAULTS"
	"$SRC_DIR/grub-regen" 'timeout_style=hidden' 'terminal_output console'
else
	[ -f "$GRUB_BG" ] ||
		{ echo "no $GRUB_BG yet — run ~/.config/mango/scripts/switchwall.sh --noswitch first" >&2; exit 1; }
	set_kv GRUB_TERMINAL_OUTPUT gfxterm
	set_kv GRUB_GFXMODE '1920x1080,auto'
	set_kv GRUB_BACKGROUND "$GRUB_BG"
	"$SRC_DIR/grub-regen" 'timeout_style=hidden' 'terminal_output gfxterm' 'mango-bg.png'
fi

cat <<MSG

Installed ($variant). F4, Esc or held Shift during the 2 s timeout shows the menu.

Rollback:
  sudo cp $DEFAULTS.bak.$STAMP $DEFAULTS
  sudo /usr/local/bin/grub-regen
MSG
