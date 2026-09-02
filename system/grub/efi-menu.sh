#!/usr/bin/env bash
# Organize the firmware (BIOS) boot menu. Safe to re-run.
#
#   sudo ~/src/mango-dotfiles/system/grub/efi-menu.sh
#
# What it does:
#   1. Adds a "Memtest86+" entry for \memtest86+\memtest.efi when the
#      file is on the ESP and no entry points to it yet.
#   2. Sets BootOrder to: GRUB, ST GRUB (supergrub2), Memtest86+,
#      Windows Boot Manager, then everything else in its old order.
#
# supergrub2 stays reachable only from this menu, on purpose — it is
# not in grub.cfg. It loop-boots the ISOs in /boot/boot-isos; that
# stops working once Secure Boot is activated (see system/secureboot).
#
# This script changes NVRAM boot variables only. It does not touch the
# ESP, grub.cfg, or the default boot path (GRUB stays first).

set -eu

die() { printf 'efi-menu: %s\n' "$1" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run me with sudo"
command -v efibootmgr >/dev/null || die "missing dependency: efibootmgr"

esp_part=$(findmnt -no SOURCE /boot) || die "/boot is not mounted"
part_num=$(cat "/sys/class/block/$(basename "$esp_part")/partition")
disk="/dev/$(lsblk -no pkname "$esp_part")"

# entry_num LABEL — boot number for an exact label, empty when absent.
entry_num() {
	efibootmgr | sed -n "s/^Boot\([0-9A-F]\{4\}\)\*\{0,1\} $1\(\t.*\)\{0,1\}\$/\1/p" | head -n1
}

if [ -f /boot/memtest86+/memtest.efi ]; then
	if efibootmgr | grep -qi 'memtest86+\\memtest\.efi'; then
		echo "==> Memtest86+ entry already present"
	else
		echo "==> create Memtest86+ entry ($disk part $part_num)"
		efibootmgr --create --disk "$disk" --part "$part_num" \
			--label 'Memtest86+' --loader '\memtest86+\memtest.efi' >/dev/null
	fi
else
	echo "==> no /boot/memtest86+/memtest.efi — install memtest86+-efi first"
fi

echo "==> BootOrder: GRUB, ST GRUB, Memtest86+, Windows, rest"
order=""
for label in GRUB 'ST GRUB' 'Memtest86+' 'Windows Boot Manager'; do
	num=$(entry_num "$label") || true
	if [ -n "$num" ]; then order="$order,$num"; fi
done
order=${order#,}
[ -n "$order" ] || die "found none of the expected entries"
rest=$(efibootmgr | sed -n 's/^BootOrder: //p' | tr ',' '\n' |
	grep -vxF -e "${order//,/$'\n'}" | paste -sd, -) || true
if [ -n "$rest" ]; then order="$order,$rest"; fi
efibootmgr --bootorder "$order" >/dev/null
efibootmgr | grep -E '^(BootOrder|Boot[0-9A-F]{4})'
