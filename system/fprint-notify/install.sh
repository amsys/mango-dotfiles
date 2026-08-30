#!/usr/bin/env bash
# Installs the root-owned fingerprint-notify helper. Follows the
# system/powermode/ precedent: sudo-gated, nothing under system/ is
# symlinked into ~/.config, so this script is the only way it lands.
#
#   sudo ~/src/mango-dotfiles/system/fprint-notify/install.sh
#
# This script does NOT edit /etc/pam.d/sudo. It prints the lines for you
# to add by hand. A bad edit to a PAM auth stack can lock the account out
# of sudo, so a human reviews that file.
#
# /etc/pam.d/swaylock is deliberately out of scope. swaylock covers the
# whole screen, so a notification behind it is invisible.
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ICON=/usr/local/share/pixmaps/mango-fprint-notify.svg

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER to verify the fix as" >&2; exit 1; }

for b in runuser setsid notify-send makoctl mmsg jq; do
	command -v "$b" > /dev/null 2>&1 || { echo "missing dependency: $b — install it first" >&2; exit 1; }
done

# install(1) overwrites, same as every other system/*/install.sh here.
# Re-running this script is the supported way to pick up an edit.
echo "==> helper -> /usr/local/bin/mango-fprint-notify"
install -m 0755 -o root -g root "$SRC_DIR/fprint-notify.sh" /usr/local/bin/mango-fprint-notify
echo "==> icon   -> $ICON"
install -D -m 0644 -o root -g root "$SRC_DIR/fingerprint.svg" "$ICON"

echo "==> self-check (no PAM_SERVICE — must be silent and exit 0)"
/usr/local/bin/mango-fprint-notify < /dev/null || { echo "helper exited non-zero with no PAM_SERVICE — do NOT add the PAM lines" >&2; exit 1; }

echo "==> test notification as $TARGET_USER (persists ~4s, then auto-dismisses)"
PAM_SERVICE=install-test PAM_RUSER="$TARGET_USER" /usr/local/bin/mango-fprint-notify < /dev/null
# The worker watches this installer as the "sudo" caller: the notification
# must appear now and disappear right after this script exits — the same
# lifecycle a real fingerprint request gets.
sleep 4

cat << 'EOF'

Installed. One manual step is left — add BOTH lines to /etc/pam.d/sudo:

  auth    optional   pam_exec.so quiet /usr/local/bin/mango-fprint-notify   <- ABOVE pam_fprintd.so
  account optional   pam_exec.so quiet /usr/local/bin/mango-fprint-notify   <- ABOVE 'account include system-auth'

The auth line shows the notification before pam_fprintd waits for a
finger; `optional` means it can never fail an authentication. The account
line runs after a successful auth and dismisses the notification — that
is how it disappears the moment the fingerprint is accepted. Without it
the notification still goes away when sudo exits, or after ~35s.

Keep a second terminal with an open root shell while you edit
/etc/pam.d/sudo. A broken auth stack there means no more sudo.

The persistent style and the click action live in the mako rule
[app-name=fprint] (matugen template mako/config). Click = jump to the
window that asked; the notification stays until the request ends.

Test:

  sudo -k && sudo true     # notification with tag info, gone once the finger is read

Rollback:
  remove both pam_exec lines from /etc/pam.d/sudo
  sudo rm /usr/local/bin/mango-fprint-notify
EOF
