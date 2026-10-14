#!/usr/bin/env bash
# Install the lock screen's root side: a hyprlock PAM stack without
# faillock, and the sudo rule that shows the TPM TOTP. Safe to re-run.
#
#   sudo ~/projects/system/mango-dotfiles/src/system/lockscreen/install.sh
#
# Two files:
#
#   /etc/pam.d/hyprlock        pam_unix with a 3 s fail delay, no faillock.
#                              Wrong attempts at the lock screen no longer
#                              lock SDDM, the TTYs and sudo for 10 minutes.
#   /etc/sudoers.d/mango-totp  `tpm2-totp calculate` without a password,
#                              for mango/scripts/lock-status.sh.
#
# The TOTP needs system/tpm-totp first (a sealed secret). Without it the
# lock screen shows the date, and fingerprint login stays off.
#
# /etc/pam.d/hyprlock belongs to the hyprlock package. pacman keeps this
# copy on upgrade and writes hyprlock.pacnew next to it. Re-run this script
# after you merge a .pacnew.
#
# Before the first lock after this install, keep a root shell open on a
# TTY. If the new stack refuses your password, `pkill -USR1 hyprlock` as
# root opens the session, and the backup puts the old stack back:
#
#   cp /etc/pam.d/hyprlock.bak.<stamp> /etc/pam.d/hyprlock

# check: /usr/local/share/mango-lockscreen/pam.d-hyprlock
# risk: boot
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"
SHARE=/usr/local/share/mango-lockscreen

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly: the sudo rule is tested as \$SUDO_USER" >&2; exit 1; }
for cmd in visudo runuser tpm2-totp hyprlock; do
	command -v "$cmd" >/dev/null || { echo "missing dependency: $cmd" >&2; exit 1; }
done
for mod in pam_faildelay.so pam_unix.so; do
	[ -e "/usr/lib/security/$mod" ] || { echo "missing PAM module: $mod" >&2; exit 1; }
done

echo "==> sudoers rule -> /etc/sudoers.d/mango-totp"
visudo -c -q -f "$SRC_DIR/sudoers.d-mango-totp"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-mango-totp" /etc/sudoers.d/mango-totp
visudo -c -q || { rm -f /etc/sudoers.d/mango-totp; echo "sudoers no longer parses: rule removed" >&2; exit 1; }

echo "==> self-check: tpm2-totp calculate as $TARGET_USER, no password"
out=$(runuser -u "$TARGET_USER" -- sudo -n /usr/bin/tpm2-totp calculate 2>&1) || true
case "$out" in
[0-9][0-9][0-9][0-9][0-9][0-9]) echo "    ok: the TPM gave a code" ;;
sudo:*)
	echo "    the rule does not apply: $out" >&2
	echo "    (a later file in /etc/sudoers.d may override it)" >&2
	exit 1
	;;
*) echo "    sudo works, but the TPM gave no code: $out" ;;
esac

echo "==> PAM stack -> /etc/pam.d/hyprlock"
if cmp -s "$SRC_DIR/pam.d-hyprlock" /etc/pam.d/hyprlock; then
	echo "    already current, skipping"
else
	[ -e /etc/pam.d/hyprlock ] && cp -a /etc/pam.d/hyprlock "/etc/pam.d/hyprlock.bak.$STAMP"
	install -m 0644 -o root -g root "$SRC_DIR/pam.d-hyprlock" /etc/pam.d/hyprlock
fi
install -D -m 0644 -o root -g root "$SRC_DIR/pam.d-hyprlock" "$SHARE/pam.d-hyprlock"

cat <<MSG

Installed. Test it before you walk away:

  1. Keep a root shell open on a TTY (Ctrl+Alt+F2).
  2. Lock with SUPER+L. Type a wrong password once, then the right one.
  3. If the right one fails: on the TTY, run 'pkill -USR1 hyprlock', then
     cp /etc/pam.d/hyprlock.bak.$STAMP /etc/pam.d/hyprlock
MSG
