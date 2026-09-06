#!/usr/bin/env bash
# Installs the root-owned hotspot helper. Follows the system/powermode/
# precedent exactly: sudo-gated, nothing under system/ is symlinked into
# ~/.config, so this script is the only way any of it lands.
#
#   sudo ~/src/mango-dotfiles/system/hotspot/install.sh

# check: /usr/local/bin/mango-hotspot
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER to verify the fix as" >&2; exit 1; }

for b in hostapd dnsmasq; do
	command -v "$b" >/dev/null 2>&1 || { echo "missing dependency: $b — install it first (pacman -S $b)" >&2; exit 1; }
done

echo "==> helper -> /usr/local/bin/mango-hotspot"
install -m 0755 -o root -g root "$SRC_DIR/mango-hotspot" /usr/local/bin/mango-hotspot

echo "==> self-check"
/usr/local/bin/mango-hotspot test || { echo "mango-hotspot test failed — not installing the sudoers rule" >&2; exit 1; }

echo "==> sudoers rule -> /etc/sudoers.d/mango-hotspot"
visudo -c -q -f "$SRC_DIR/sudoers.d-mango-hotspot"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-mango-hotspot" /etc/sudoers.d/mango-hotspot

echo "==> verifying as $TARGET_USER"
if sudo -n -u "$TARGET_USER" sudo -n /usr/local/bin/mango-hotspot down </dev/null >/dev/null 2>&1; then
	echo "    mango-hotspot down runs with no password prompt"
else
	echo "    still prompting for $TARGET_USER. Check:" >&2
	echo "      getent group wheel                     # is $TARGET_USER a member?" >&2
	echo "      sudo -l -U $TARGET_USER                 # should list mango-hotspot up/down" >&2
	exit 1
fi

cat <<EOF

Installed. Next:

  ~/.config/ironbar/scripts/hotspot.sh test     # self-check
  ~/.config/ironbar/scripts/hotspot.sh --toggle # turn the hotspot on/off

Rollback:
  sudo rm /usr/local/bin/mango-hotspot /etc/sudoers.d/mango-hotspot
EOF
