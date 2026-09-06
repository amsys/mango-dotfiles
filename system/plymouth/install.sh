#!/usr/bin/env bash
# Install the mango plymouth theme: the LUKS prompt that matches the SDDM
# greeter. Safe to re-run after any change under system/plymouth/.
#
#   sudo ~/src/mango-dotfiles/system/plymouth/install.sh
#
# Ordered so everything that only lays files down comes first, the two edits
# that change what boots (HOOKS and the grub defaults + grub.cfg) next, and
# the one slow step that bakes them in (`mkinitcpio -P`) last. Abandoning the
# script before that point changes nothing about the next boot.
#
# The theme's colours, wallpaper and glyphs are not in the initramfs at all:
# they travel as PNGs in the early initrd /boot/mango-plymouth.img, rebuilt
# by /usr/local/bin/plymouth-theme-sync on each wallpaper switch. The host
# copy of the theme directory must never gain a dyn/ subdirectory — the
# stock plymouth hook copies the whole directory, and a stale dyn/ baked into
# the main image would shadow the early initrd forever.
#
# When plymouth is not running, systemd falls back to a plain text console
# password agent — no themed fallback prompt here; mango-cryptbox, which
# used to theme it, is gone.
#
# The handover to SDDM is plymouth's plain `quit`, plus an sddm.service
# drop-in that waits for plymouth-quit-wait. `quit --retain-splash` is not
# used: SDDM has no plymouth handover, and with the splash retained X hung
# before it opened /dev/dri/card1 on every boot (i915, 2026-09-01).

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SRC_DIR/../.." && pwd)"
THEME_DIR=/usr/share/plymouth/themes/mango
DEFAULTS=/etc/default/grub
STAMP="$(date +%Y%m%d-%H%M%S)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
for cmd in plymouthd plymouth-set-default-theme mkinitcpio grub-mkconfig magick cpio visudo fc-match file; do
	command -v "$cmd" >/dev/null || { echo "missing dependency: $cmd" >&2; exit 1; }
done
[ -f /usr/lib/initcpio/install/plymouth ] || { echo "plymouth's mkinitcpio hook is missing" >&2; exit 1; }
grep -qE '\bsd-encrypt\b' /etc/mkinitcpio.conf ||
	{ echo "no 'sd-encrypt' hook found in HOOKS — add plymouth to HOOKS by hand" >&2; exit 1; }
if [ -e "$THEME_DIR/dyn" ]; then
	echo "$THEME_DIR/dyn exists — remove it first; dynamic assets must never live in the host theme dir" >&2
	exit 1
fi

echo "==> theme -> $THEME_DIR"
install -d -m 0755 "$THEME_DIR"
install -m 0644 -o root -g root "$SRC_DIR/theme/mango.plymouth" "$SRC_DIR/theme/mango.script" "$THEME_DIR/"

echo "==> sync tool -> /usr/local/bin/plymouth-theme-sync"
install -m 0755 -o root -g root "$SRC_DIR/plymouth-theme-sync" /usr/local/bin/plymouth-theme-sync

echo "==> sudoers rule -> /etc/sudoers.d/plymouth-theme-sync"
visudo -c -q -f "$SRC_DIR/sudoers.d-plymouth-theme-sync"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-plymouth-theme-sync" /etc/sudoers.d/plymouth-theme-sync

echo "==> /etc/plymouth/plymouthd.conf"
[ -f /etc/plymouth/plymouthd.conf ] && cp -a /etc/plymouth/plymouthd.conf "/etc/plymouth/plymouthd.conf.bak.$STAMP"
install -D -m 0644 -o root -g root "$SRC_DIR/plymouthd.conf" /etc/plymouth/plymouthd.conf
[ "$(plymouth-set-default-theme)" = mango ] || { echo "plymouth does not see the mango theme" >&2; exit 1; }

echo "==> sddm drop-in -> /etc/systemd/system/sddm.service.d/10-mango.conf"
install -D -m 0644 -o root -g root "$SRC_DIR/sddm-after-plymouth.conf" \
	/etc/systemd/system/sddm.service.d/10-mango.conf
if [ -e /etc/systemd/system/plymouth-quit.service.d/10-mango.conf ]; then
	echo "    removing the retired plymouth-quit --retain-splash drop-in"
	rm -f /etc/systemd/system/plymouth-quit.service.d/10-mango.conf
	rmdir /etc/systemd/system/plymouth-quit.service.d 2>/dev/null || true
fi

echo "==> gap frame unit -> /etc/systemd/system/mango-fb-background.service"
install -m 0644 -o root -g root "$SRC_DIR/mango-fb-background.service" \
	/etc/systemd/system/mango-fb-background.service
systemctl daemon-reload
systemctl enable -q mango-fb-background.service

# matugen has not necessarily rendered yet: seed neutral assets so the first
# boot after install shows the prompt, and so grub-mkconfig (which silently
# drops a missing early initrd) sees the file.
# The second condition catches an already-installed machine that predates
# the shutdown splash: seed renders and installs both halves.
if [ ! -f /boot/mango-plymouth.img ] || [ ! -d "$THEME_DIR/shutdown" ]; then
	echo "==> seeding a neutral /boot/mango-plymouth.img (matugen will replace it)"
	/usr/local/bin/plymouth-theme-sync seed
fi

echo "==> /etc/mkinitcpio.conf"
if grep -qE '\bplymouth\b' /etc/mkinitcpio.conf; then
	echo "    plymouth hook already present, skipping"
else
	cp -a /etc/mkinitcpio.conf "/etc/mkinitcpio.conf.bak.$STAMP"
	sed -i -E 's/\bsd-encrypt\b/plymouth sd-encrypt/' /etc/mkinitcpio.conf
	grep -qE '\bplymouth\b' /etc/mkinitcpio.conf ||
		{ echo "could not add plymouth to HOOKS" >&2; exit 1; }
fi
grep -qE '\bkms\b' /etc/mkinitcpio.conf ||
	echo "    warning: no kms hook in HOOKS — plymouth will start on simpledrm only"

echo "==> $DEFAULTS (backup: $DEFAULTS.bak.$STAMP)"
cp -a "$DEFAULTS" "$DEFAULTS.bak.$STAMP"
if grep -q '^GRUB_EARLY_INITRD_LINUX_CUSTOM=' "$DEFAULTS"; then
	sed -i 's|^GRUB_EARLY_INITRD_LINUX_CUSTOM=.*|GRUB_EARLY_INITRD_LINUX_CUSTOM="mango-plymouth.img"|' "$DEFAULTS"
else
	printf '\nGRUB_EARLY_INITRD_LINUX_CUSTOM="mango-plymouth.img"\n' >>"$DEFAULTS"
fi
if grep -qE '^GRUB_CMDLINE_LINUX_DEFAULT=".*\bsplash\b' "$DEFAULTS"; then
	echo "    splash already on the kernel command line"
else
	sed -i -E 's/^GRUB_CMDLINE_LINUX_DEFAULT="(.*)"$/GRUB_CMDLINE_LINUX_DEFAULT="\1 splash"/' "$DEFAULTS"
	grep -qE '^GRUB_CMDLINE_LINUX_DEFAULT=".*\bsplash\b' "$DEFAULTS" ||
		{ echo "could not add splash to GRUB_CMDLINE_LINUX_DEFAULT" >&2; exit 1; }
fi

echo "==> regenerating /boot/grub/grub.cfg"
"$REPO_DIR/system/grub/grub-regen" 'mango-plymouth.img' 'splash'

echo "==> mkinitcpio -P (the only slow step)"
mkinitcpio -P

# plymouthd's script plugin crashes at boot when its label fonts are missing
# (seen in a VM: a crash here leaves the LUKS prompt with no password agent).
# The stock hook copies them from fc-match; make sure it did.
echo "==> checking the image"
for want in usr/share/fonts/Plymouth.ttf usr/share/fonts/Plymouth-monospace.ttf \
	usr/share/plymouth/themes/mango/mango.script usr/lib/plymouth/script.so; do
	lsinitcpio /boot/initramfs-linux.img | grep -qx "$want" ||
		{ echo "$want is missing from /boot/initramfs-linux.img — do not reboot on this image; rollback below" >&2; exit 1; }
done
if lsinitcpio /boot/initramfs-linux.img | grep -q 'themes/mango/dyn/'; then
	echo "the image contains themes/mango/dyn/ — it must not; rollback below" >&2
	exit 1
fi

cat <<MSG

Installed. Next:

  ~/.config/mango/scripts/switchwall.sh --noswitch     # render the real assets
  cpio -t < /boot/mango-plymouth.img                   # should list dyn/*.png
  ls /usr/share/plymouth/themes/mango/shutdown         # 4 PNGs, the shutdown splash
  lsinitcpio /boot/initramfs-linux.img | grep -c themes/mango/dyn   # must be 0
  reboot                                               # the only real test
  (plymouth.enable=0 on the kernel line brings the console prompt back;
   F4 at the GRUB countdown shows the rescue entries)

Rollback:
  sudo cp $DEFAULTS.bak.$STAMP $DEFAULTS
  sudo cp /etc/mkinitcpio.conf.bak.$STAMP /etc/mkinitcpio.conf   # if a backup was made
  sudo rm -f /boot/mango-plymouth.img /etc/systemd/system/sddm.service.d/10-mango.conf
  sudo systemctl disable -q mango-fb-background.service
  sudo rm -f /etc/systemd/system/mango-fb-background.service
  sudo /usr/local/bin/grub-regen
  sudo mkinitcpio -P
MSG
