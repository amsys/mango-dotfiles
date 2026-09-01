#!/usr/bin/env bash
# Pin the kernel and initramfs that booted this machine as a GRUB rescue
# entry. Safe to re-run: each run re-pins whatever is in /boot now.
#
#   sudo ~/src/mango-dotfiles/system/boot-pin/pin-kernel.sh [--force]
#
# What survives what:
#   - a `linux` package upgrade replaces /boot/vmlinuz-linux and rebuilds
#     /boot/initramfs-linux.img; the copies in /boot/pinned/ are not touched.
#   - grub-mkconfig always includes /etc/grub.d/40_custom verbatim, and
#     pacman never overwrites it (backup file), so the entry survives regens.
#   - the copies live in /boot/pinned/, not /boot: /etc/grub.d/10_linux
#     globs /boot/vmlinuz-* and sorts "linux-pinned" above "linux", so a
#     pinned copy in /boot becomes the default entry and the machine keeps
#     booting the old kernel after every upgrade (seen 2026-09-01).
#   - the pinned entry has no `splash`: plymouth runs its text prompt there,
#     with the attestation code on the top line. The verbose entry has
#     `plymouth.enable=0`: systemd's plain text console prompt.
#
# --force: pin even when /boot/vmlinuz-linux is not the running kernel
# (default refuses: an untested kernel is not a rescue).

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SRC_DIR/../.." && pwd)"
CFG=/boot/grub/grub.cfg
CUSTOM=/etc/grub.d/40_custom
PIN_DIR=/boot/pinned
MIN_FREE_MB=80
force=0

case "${1:-}" in
	'') ;;
	--force) force=1 ;;
	*) echo "usage: $0 [--force]" >&2; exit 1 ;;
esac

die() {
	printf 'pin-kernel: %s\n' "$1" >&2
	exit 1
}

[ "$(id -u)" -eq 0 ] || die "run me with sudo"
[ -f "$CFG" ] || die "$CFG not found"
[ -f /boot/vmlinuz-linux ] && [ -f /boot/initramfs-linux.img ] ||
	die "/boot/vmlinuz-linux or /boot/initramfs-linux.img missing"

kver=$(file -b /boot/vmlinuz-linux | grep -oE 'version [^ ]+' | cut -d' ' -f2)
[ -n "$kver" ] || die "could not read the kernel version from /boot/vmlinuz-linux"
if [ "$kver" != "$(uname -r)" ] && [ "$force" -eq 0 ]; then
	die "/boot/vmlinuz-linux is $kver, running kernel is $(uname -r) — reboot into it first, or --force"
fi

free_mb=$(df --output=avail -BM /boot | tail -n1 | tr -dc '0-9')
[ "$free_mb" -ge "$MIN_FREE_MB" ] || die "only ${free_mb} MB free on /boot, need $MIN_FREE_MB"

# The prelude and the linux line come from the entry that boots today, not
# from a hard-coded copy: partition UUIDs and cmdline stay in one place.
body=$(awk "/^menuentry 'Arch Linux' /{f=1;next} f&&/^}/{exit} f" "$CFG")
[ -n "$body" ] || die "no \"menuentry 'Arch Linux'\" in $CFG"
linux_line=$(printf '%s\n' "$body" | grep -E '^\s*linux\s' | head -n1 |
	sed -E 's|/vmlinuz-linux-pinned([[:space:]])|/vmlinuz-linux\1|')
[ -n "$linux_line" ] || die "no linux line in the Arch Linux entry"
printf '%s\n' "$linux_line" | grep -qE '/vmlinuz-linux[[:space:]]' ||
	die "the Arch Linux entry does not boot /vmlinuz-linux: $linux_line"
prelude=$(printf '%s\n' "$body" | grep -vE '^\s*(linux|initrd|echo)\s')
pinned_linux=$(printf '%s\n' "$linux_line" |
	sed -E 's|/vmlinuz-linux([[:space:]])|/pinned/vmlinuz-linux-pinned\1|; s/[[:space:]]+splash([[:space:]]|$)/\1/')
printf '%s\n' "$pinned_linux" | grep -q 'pinned/vmlinuz-linux-pinned' || die "could not rewrite the linux line"
# The verbose entry boots the *current* kernel with every message on screen:
# no quiet, no splash, no loglevel cap, plymouth off (console LUKS prompt).
verbose_linux=$(printf '%s\n' "$linux_line" |
	sed -E 's/[[:space:]]+(quiet|splash|loglevel=[0-9]+)([[:space:]]|$)/\2/g; s/[[:space:]]+(quiet|splash|loglevel=[0-9]+)([[:space:]]|$)/\2/g')
verbose_linux="$verbose_linux plymouth.enable=0"

echo "==> $PIN_DIR/vmlinuz-linux-pinned, $PIN_DIR/initramfs-linux-pinned.img ($kver)"
install -d -m 0755 "$PIN_DIR"
for pair in "vmlinuz-linux:vmlinuz-linux-pinned" "initramfs-linux.img:initramfs-linux-pinned.img"; do
	src=/boot/${pair%%:*}
	dst=$PIN_DIR/${pair##*:}
	cp -f -- "$src" "$dst.new"
	cmp -s -- "$src" "$dst.new" || { rm -f "$dst.new"; die "copy of $src does not match"; }
	mv -f -- "$dst.new" "$dst"
done
# Copies from before 2026-09-01 sat in /boot itself, inside 10_linux's glob.
for old in /boot/vmlinuz-linux-pinned /boot/initramfs-linux-pinned.img; do
	[ -e "$old" ] || continue
	echo "==> removing $old (moved to $PIN_DIR/)"
	rm -f -- "$old"
done

echo "==> $CUSTOM (mango-pin block)"
block=$(cat <<BLOCK
# BEGIN mango-pin (written by system/boot-pin/pin-kernel.sh, do not edit)
menuentry 'Arch Linux (pinned $kver, $(date +%F))' --class arch --class gnu-linux --class gnu --class os \$menuentry_id_option 'mango-pinned' {
$prelude
$pinned_linux
	initrd	/intel-ucode.img /pinned/initramfs-linux-pinned.img
}
menuentry 'Arch Linux (verbose console, current kernel)' --class arch --class gnu-linux --class gnu --class os \$menuentry_id_option 'mango-verbose' {
$prelude
$verbose_linux
	initrd	/intel-ucode.img /initramfs-linux.img
}
# END mango-pin
BLOCK
)
tmp=$(mktemp)
if grep -q '^# BEGIN mango-pin' "$CUSTOM"; then
	awk '/^# BEGIN mango-pin/{skip=1} !skip{print} /^# END mango-pin/{skip=0}' "$CUSTOM" >"$tmp"
else
	cat "$CUSTOM" >"$tmp"
fi
printf '%s\n' "$block" >>"$tmp"
head -n1 "$tmp" | grep -q '^#!/bin/sh' || { rm -f "$tmp"; die "$CUSTOM lost its shebang"; }
install -m 0755 -o root -g root -- "$tmp" "$CUSTOM"
rm -f "$tmp"

"$REPO_DIR/system/grub/grub-regen" 'pinned/vmlinuz-linux-pinned' "pinned $kver" 'plymouth.enable=0'

# The default entry must still be the real kernel: 10_linux must not have
# adopted a pinned copy.
awk "/^menuentry 'Arch Linux' /{f=1;next} f&&/^}/{exit} f" "$CFG" |
	grep -qE '^\s*linux\s+/vmlinuz-linux\s' ||
	die "the default 'Arch Linux' entry no longer boots /vmlinuz-linux — check $CFG"

cat <<MSG

Pinned $kver, plus a verbose-console entry for the current kernel.
Both show in the GRUB menu (F4, Esc or held Shift during the timeout).

Rollback:
  sudo sed -i '/^# BEGIN mango-pin/,/^# END mango-pin/d' $CUSTOM
  sudo rm -rf $PIN_DIR
  sudo /usr/local/bin/grub-regen
MSG
