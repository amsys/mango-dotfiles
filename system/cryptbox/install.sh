#!/usr/bin/env bash
# Install mango-cryptbox: a matugen-coloured ASCII frame around the LUKS
# unlock prompt. Safe to re-run after any change under system/cryptbox/.
#
#   sudo ~/src/mango-dotfiles/system/cryptbox/install.sh
#
# Ordered so the two steps that change what actually boots — the mkinitcpio.conf
# edit and the grub.cfg regeneration — happen before the one expensive step
# that bakes them in (`mkinitcpio -P`). Everything before that point is safe
# to abandon; nothing after it runs.
#
# Why an early initrd and not a plymouth theme: plymouth's theme lives inside
# the initramfs, so recolouring it costs `mkinitcpio -P` on every wallpaper
# switch — tens of seconds and a 30 MB rewrite of /boot for a screen on
# screen for about three seconds, once per boot. The early initrd this script
# sets up is the same mechanism Arch uses to deliver microcode: the kernel
# unpacks it into the initramfs rootfs before init runs, so a wallpaper
# switch only ever costs an atomic ~1 KB write to /boot/mango-palette.img —
# see console-palette-sync. `mkinitcpio -P` runs exactly once, here.

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"
DROPIN_DIR=/usr/local/share/mango-cryptbox

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
for cmd in cpio visudo mkinitcpio grub-mkconfig stty; do
	command -v "$cmd" >/dev/null || { echo "missing dependency: $cmd" >&2; exit 1; }
done

echo "==> mango-cryptbox -> /usr/local/bin/mango-cryptbox"
install -m 0755 -o root -g root "$SRC_DIR/mango-cryptbox" /usr/local/bin/mango-cryptbox

echo "==> sync tool -> /usr/local/bin/console-palette-sync"
install -m 0755 -o root -g root "$SRC_DIR/console-palette-sync" /usr/local/bin/console-palette-sync

echo "==> systemd drop-in source -> $DROPIN_DIR/ask-password-dropin.conf"
install -D -m 0644 -o root -g root "$SRC_DIR/ask-password-dropin.conf" \
	"$DROPIN_DIR/ask-password-dropin.conf"

echo "==> mkinitcpio hook -> /usr/lib/initcpio/install/mango-cryptbox"
install -m 0755 -o root -g root "$SRC_DIR/mkinitcpio-install" \
	/usr/lib/initcpio/install/mango-cryptbox

echo "==> sudoers rule -> /etc/sudoers.d/console-palette-sync"
visudo -c -q -f "$SRC_DIR/sudoers.d-console-palette-sync"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-console-palette-sync" \
	/etc/sudoers.d/console-palette-sync

# matugen has not necessarily rendered a palette yet. Seed a neutral grey
# early initrd directly — bypassing console-palette-sync, which expects to
# run through sudo from a logged-in user's own render — so the first boot
# after install is never uncoloured.
if [ ! -f /boot/mango-palette.img ]; then
	echo "==> seeding a neutral /boot/mango-palette.img (matugen will replace it)"
	tmp="$(mktemp -d)"
	i=0
	while [ "$i" -le 15 ]; do
		echo "$i 808080"
		i=$((i + 1))
	done >"$tmp/palette.conf"
	install -Dm644 -- "$tmp/palette.conf" "$tmp/etc/mango/palette.conf"
	(cd "$tmp" && printf 'etc/mango/palette.conf\n' | cpio --quiet -o -H newc) \
		>/boot/mango-palette.img
	rm -rf "$tmp"
fi

echo "==> /etc/mkinitcpio.conf"
if grep -q 'mango-cryptbox' /etc/mkinitcpio.conf; then
	echo "    already present, skipping"
else
	cp -a /etc/mkinitcpio.conf "/etc/mkinitcpio.conf.bak.$STAMP"
	sed -i 's/\bsd-encrypt\b/mango-cryptbox sd-encrypt/' /etc/mkinitcpio.conf
	grep -q 'mango-cryptbox' /etc/mkinitcpio.conf ||
		{ echo "no 'sd-encrypt' hook found in HOOKS — add mango-cryptbox to HOOKS by hand" >&2; exit 1; }
fi

echo "==> /etc/default/grub (GRUB_EARLY_INITRD_LINUX_CUSTOM)"
cp -a /etc/default/grub "/etc/default/grub.bak.$STAMP"
if grep -q '^GRUB_EARLY_INITRD_LINUX_CUSTOM=' /etc/default/grub; then
	sed -i 's|^GRUB_EARLY_INITRD_LINUX_CUSTOM=.*|GRUB_EARLY_INITRD_LINUX_CUSTOM="mango-palette.img"|' \
		/etc/default/grub
else
	printf '\nGRUB_EARLY_INITRD_LINUX_CUSTOM="mango-palette.img"\n' >>/etc/default/grub
fi

echo "==> regenerating /boot/grub/grub.cfg"
new_cfg="$(mktemp)"
grub-mkconfig -o "$new_cfg"
grep -q 'vmlinuz-linux' "$new_cfg" ||
	{ echo "grub-mkconfig produced no kernel entry — not installing it" >&2; exit 1; }
grep -q 'mango-palette.img' "$new_cfg" ||
	{ echo "grub-mkconfig did not pick up the early initrd — not installing it" >&2; exit 1; }
cp -a /boot/grub/grub.cfg "/boot/grub/grub.cfg.bak.$STAMP"
install -m 0644 -- "$new_cfg" /boot/grub/grub.cfg
rm -f "$new_cfg"

echo "==> mkinitcpio -P (the only slow step)"
mkinitcpio -P

cat <<EOF

Installed. Next:

  ~/.config/mango/scripts/switchwall.sh --noswitch     # pull in the real colours
  mango-cryptbox --demo                                # eyeball the frame first
  reboot                                                # the only real test

Rollback:
  sudo rm -f /boot/mango-palette.img
  sudo cp /etc/mkinitcpio.conf.bak.$STAMP /etc/mkinitcpio.conf
  sudo cp /etc/default/grub.bak.$STAMP /etc/default/grub
  sudo cp /boot/grub/grub.cfg.bak.$STAMP /boot/grub/grub.cfg
  sudo mkinitcpio -P
EOF
