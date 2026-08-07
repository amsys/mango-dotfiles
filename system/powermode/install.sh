#!/usr/bin/env bash
# Installs the root-owned power-mode helper. Follows the system/rapl/
# precedent exactly: sudo-gated, nothing under system/ is symlinked into
# ~/.config, so this script is the only way any of it lands.
#
#   sudo ~/src/mango-dotfiles/system/powermode/install.sh
#
# mango/powermode.conf (the values) is a plain tracked, symlinked file and
# needs NO reinstall when edited — only mango-powermode itself (this script)
# needs a reinstall if changed.

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER to verify the fix as" >&2; exit 1; }

echo "==> helper -> /usr/local/bin/mango-powermode"
install -m 0755 -o root -g root "$SRC_DIR/mango-powermode" /usr/local/bin/mango-powermode

echo "==> self-check"
/usr/local/bin/mango-powermode test || { echo "mango-powermode test failed — not installing the sudoers rule" >&2; exit 1; }

echo "==> sudoers rule -> /etc/sudoers.d/mango-powermode"
visudo -c -q -f "$SRC_DIR/sudoers.d-mango-powermode"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-mango-powermode" /etc/sudoers.d/mango-powermode

echo "==> verifying as $TARGET_USER"
if sudo -n -u "$TARGET_USER" sudo -n /usr/local/bin/mango-powermode apply < /dev/null > /dev/null 2>&1; then
	echo "    mango-powermode apply runs with no password prompt"
else
	echo "    still prompting for $TARGET_USER. Check:" >&2
	echo "      getent group wheel                          # is $TARGET_USER a member?" >&2
	echo "      sudo -l -U $TARGET_USER                      # should list mango-powermode apply" >&2
	exit 1
fi

cat << EOF

Installed. Next:

  ~/.config/mango/scripts/powermode.sh status   # should print the current mode
  ~/.config/mango/scripts/powermode.sh full     # apply full performance now
  ~/.config/mango/scripts/powermode.sh eco      # apply eco now

powermode.conf lives at ~/.config/mango/powermode.conf (tracked, symlinked) —
edit it and switch modes to pick up a changed value, no reinstall needed.

Rollback:
  sudo rm /usr/local/bin/mango-powermode /etc/sudoers.d/mango-powermode
EOF
