#!/usr/bin/env bash
# Prepare TPM-attested Secure Boot. Safe to re-run. This script ACTIVATES
# NOTHING: it creates keys, signs files, and stages a second GRUB image.
# Secure Boot stays off, no firmware variable changes, the live loader
# (Boot0001 -> \EFI\GRUB\grubx64.efi) is untouched. Activation is the
# manual step list this script prints at the end.
#
#   sudo ~/src/mango-dotfiles/system/secureboot/install.sh
#
# The chain it prepares:
#   firmware (our PK/KEK/db via sbctl) verifies -> GRUB-SB image
#   GRUB-SB (embedded GPG key, check_signatures=enforce) verifies ->
#   grub.cfg, grubenv, modules, font, mango-bg.png, kernel, intel-ucode,
#   mango-plymouth.img, initramfs, pinned kernels — every file it loads.
#   GRUB's tpm module measures the same files into PCR 8/9.
#
# What it does:
#   1. GPG signing key (RSA-4096, no passphrase, root-only) in
#      /etc/mango-secureboot/gnupg. No passphrase because pacman hooks and
#      mkinitcpio must sign unattended; the key sits on the encrypted
#      root, and it only protects files on the unencrypted ESP.
#   2. sbctl create-keys (PK/KEK/db). Created, NOT enrolled.
#   3. mango-sign-boot -> /usr/local/bin (all signing logic).
#   4. mkinitcpio post script + pacman hook, so signatures stay fresh
#      through kernel updates and manual mkinitcpio runs. Both are
#      harmless while Secure Boot is off — stale or missing .sig files
#      change nothing until the GRUB-SB image is the boot path.
#   5. Stage /boot/EFI/GRUB-SB/grubx64.efi (grub-install --pubkey
#      --disable-shim-lock --modules=tpm --no-nvram) and sign everything.
#   6. sbctl-sign the other EFI binaries on the ESP (live grubx64.efi,
#      bootx64.efi fallback, supergrub2.efi) so they stay bootable under
#      Secure Boot during burn-in. NOTE: while these stay signed, the
#      chain is not tamper-proof — the live loader does not verify what
#      it loads. Closing that hatch is the last activation step.
#
# GRUB config regeneration (grub-regen) and wallpaper switches
# (plymouth-theme-sync rewrites mango-bg.png + mango-plymouth.img) both
# call mango-sign-boot when it is installed, so those files keep valid
# signatures too.
#
# Windows (Boot0007) boots directly from the firmware, not through GRUB;
# it keeps working if activation enrolls Microsoft's certificates
# (sbctl enroll-keys -m). Loop-booting ISOs from /boot/boot-isos through
# supergrub will NOT work under Secure Boot; use a signed USB installer.

# check: /usr/local/bin/mango-sign-boot
# risk: boot
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETC=/etc/mango-secureboot
export GNUPGHOME=$ETC/gnupg
PUBKEY=$ETC/grub-pubkey.gpg

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
for cmd in sbctl grub-install gpg; do
	command -v "$cmd" >/dev/null || { echo "missing dependency: $cmd" >&2; exit 1; }
done
[ -d /sys/firmware/efi ] || { echo "not an EFI boot" >&2; exit 1; }

echo "==> GPG boot-signing key in $GNUPGHOME"
if [ -f "$PUBKEY" ]; then
	echo "    already present, skipping"
else
	install -d -m 0700 -o root -g root "$ETC" "$GNUPGHOME"
	gpg --batch --gen-key <<-EOF
		%no-protection
		Key-Type: RSA
		Key-Length: 4096
		Key-Usage: sign
		Name-Real: mango boot signing
		Expire-Date: 0
		%commit
	EOF
	gpg --export >"$PUBKEY"
	[ -s "$PUBKEY" ] || { echo "GPG key export failed" >&2; exit 1; }
fi

echo "==> sbctl keys (created only — enrolling is an activation step)"
if sbctl status 2>/dev/null | grep -q 'Installed:.*sbctl is installed'; then
	echo "    already present, skipping"
else
	sbctl create-keys
fi

echo "==> mango-sign-boot -> /usr/local/bin/mango-sign-boot"
install -m 0755 -o root -g root "$SRC_DIR/mango-sign-boot" /usr/local/bin/mango-sign-boot

echo "==> mkinitcpio post script -> /etc/initcpio/post/mango-sign-boot"
install -D -m 0755 -o root -g root "$SRC_DIR/initcpio-post" /etc/initcpio/post/mango-sign-boot

echo "==> pacman hook -> /etc/pacman.d/hooks/zz-mango-secureboot.hook"
install -D -m 0644 -o root -g root "$SRC_DIR/zz-mango-secureboot.hook" \
	/etc/pacman.d/hooks/zz-mango-secureboot.hook

echo "==> stage GRUB-SB image and sign the boot files"
/usr/local/bin/mango-sign-boot --stage-grub

echo "==> sbctl-sign the other ESP binaries (burn-in escape hatches)"
for efi in /boot/EFI/GRUB/grubx64.efi /boot/EFI/Boot/bootx64.efi \
	/boot/EFI/GRUB2/supergrub2.efi; do
	[ -f "$efi" ] || continue
	sbctl list-files | grep -qF "$efi" && sbctl sign "$efi" || sbctl sign -s "$efi"
done

cat <<'MSG'

Prepared. Nothing is active: Secure Boot is off, the boot path is unchanged.

Test now, with Secure Boot still off (recommended before any activation):
  1. Reboot into the firmware boot menu -> Boot From File ->
     \EFI\GRUB-SB\grubx64.efi. The normal menu must appear and Arch must
     boot. In the GRUB console (c), `set` must show check_signatures=enforce.
  2. Tamper test: sudo mv /boot/initramfs-linux.img.sig /root/ and boot
     GRUB-SB again — the entry must refuse to load. Move the .sig back.

Activate, one step per boot, when ready:
  1. sudo sbctl enroll-keys -m       # -m keeps Microsoft certs in db so
                                     # Windows (Boot0007) still boots.
                                     # Setup Mode ends here.
  2. Firmware setup: set Secure Boot to Enabled and set a firmware
     administrator password (without one, anyone can turn Secure Boot
     back off — the TOTP would catch it via PCR 7, but only if you read
     it). Boot GRUB-SB from the firmware menu; `bootctl status` must say
     Secure Boot: enabled.
  3. sudo efibootmgr --create --disk /dev/nvme0n1 --part 1 \
       --label GRUB-SB --loader '\EFI\GRUB-SB\grubx64.efi'
     and put it first in BootOrder.
  4. sudo tpm2-totp -P - -p 0,2,4,7 reseal
     PCR 7 changed with SB on. Run this ONLY in a boot that went through
     GRUB-SB: reseal binds the current PCR values, and PCR 4 is the hash
     of the loader the firmware started. Resealed in an old-loader boot,
     the TOTP would fail on every GRUB-SB boot and verify the wrong
     loader instead.
     The same reseal is due after every `linux` upgrade: GRUB loads the
     kernel with firmware LoadImage under Secure Boot, so a new kernel
     moves PCR 4 too. Nothing reseals for you.
  5. After burn-in, close the escape hatch — the old loader boots but does
     not verify: sudo cp /boot/EFI/GRUB-SB/grubx64.efi \
       /boot/EFI/GRUB/grubx64.efi
     (supergrub2.efi is the same kind of hatch; remove its db signature
     with sbctl when you no longer want it.)
  6. Optional TPM auto-unlock of the root LUKS (sd-encrypt is already in
     HOOKS): sudo systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs=7 \
       /dev/disk/by-uuid/39336f50-0c1b-47e8-a24d-541a9430a52e
     then append tpm2-device=auto to rd.luks.options in
     /etc/default/grub and run grub-regen.

Rollback at any point: firmware setup -> Secure Boot: Disabled. Everything
boots as before; the .sig files and the GRUB-SB image are inert baggage.
`sbctl reset` removes the enrolled keys if you want Setup Mode back.
MSG
