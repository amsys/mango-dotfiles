#!/usr/bin/env bash
# Install the TPM TOTP display for the plymouth LUKS prompt. Safe to re-run.
#
#   sudo ~/src/mango-dotfiles/system/tpm-totp/install.sh
#
# Needs system/plymouth/install.sh first (the hook goes right after
# `plymouth` in HOOKS). The secret itself is never scripted — seal it by
# hand, password on stdin, and scan the QR code into the phone:
#
#   sudo tpm2-totp -P - -p 0,2,4,7 -l nauthiz generate
#
# PCRs 0,2,4,7: firmware, option ROMs, boot loader, Secure Boot policy. Not
# 8/9: GRUB does not measure the kernel or initrds here, and the wallpaper
# early initrds would churn PCR 9 on every switch. After a firmware update or
# turning Secure Boot on:  sudo tpm2-totp -P - -p 0,2,4,7 reseal

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
for cmd in tpm2-totp mkinitcpio; do
	command -v "$cmd" >/dev/null || { echo "missing dependency: $cmd" >&2; exit 1; }
done
[ -e /usr/lib/libtss2-tcti-device.so.0 ] || { echo "tpm2-tss device TCTI missing" >&2; exit 1; }
grep -qE '\bplymouth\b' /etc/mkinitcpio.conf ||
	{ echo "no plymouth hook in /etc/mkinitcpio.conf — run system/plymouth/install.sh first" >&2; exit 1; }

echo "==> mango-totp -> /usr/local/bin/mango-totp"
install -m 0755 -o root -g root "$SRC_DIR/mango-totp" /usr/local/bin/mango-totp

echo "==> unit source -> /usr/local/share/mango-totp/mango-totp.service"
install -D -m 0644 -o root -g root "$SRC_DIR/mango-totp.service" \
	/usr/local/share/mango-totp/mango-totp.service

echo "==> mkinitcpio hook -> /usr/lib/initcpio/install/mango-totp"
install -m 0755 -o root -g root "$SRC_DIR/mkinitcpio-install" /usr/lib/initcpio/install/mango-totp

echo "==> /etc/mkinitcpio.conf"
if grep -qE '\bmango-totp\b' /etc/mkinitcpio.conf; then
	echo "    already present, skipping"
else
	cp -a /etc/mkinitcpio.conf "/etc/mkinitcpio.conf.bak.$STAMP"
	sed -i -E 's/\bplymouth\b/plymouth mango-totp/' /etc/mkinitcpio.conf
	grep -qE '\bmango-totp\b' /etc/mkinitcpio.conf ||
		{ echo "could not add mango-totp to HOOKS" >&2; exit 1; }
fi

echo "==> mkinitcpio -P (the only slow step)"
mkinitcpio -P

cat <<MSG

Installed. Next:

  sudo tpm2-totp -P - -p 0,2,4,7 -l nauthiz generate   # once; scan the QR code
  reboot                                                 # the only real test

Rollback:
  sudo cp /etc/mkinitcpio.conf.bak.$STAMP /etc/mkinitcpio.conf   # if a backup was made
  sudo rm -f /usr/lib/initcpio/install/mango-totp
  sudo mkinitcpio -P
MSG
