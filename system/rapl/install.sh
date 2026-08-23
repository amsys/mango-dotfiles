#!/usr/bin/env bash
# Give the wheel group read access to Intel RAPL energy counters, and let
# wheel run the powertop TUI with no password. Follows the system/sddm/
# precedent: sudo-gated, and nothing under system/ is symlinked into
# ~/.config, so this script is the only way any of it lands.
#
#   sudo ~/src/mango-dotfiles/system/rapl/install.sh
#
# RAPL's energy_uj is root-only by default because of the PLATYPUS side-channel
# (CVE-2020-8694 — a local attacker can infer AES keys from package energy
# readings). This is a single-user laptop, so that risk doesn't apply here, and
# RAPL is the only root-free source of "which part of this machine is drawing
# the watts" — see udev-rapl.rules for why a udev rule was used instead of the
# more obvious tmpfiles.d z-line.

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER to verify the fix as" >&2; exit 1; }
command -v powertop >/dev/null || { echo "powertop is not installed" >&2; exit 1; }

echo "==> udev rule -> /etc/udev/rules.d/mango-rapl.rules"
install -m 0644 -o root -g root "$SRC_DIR/udev-rapl.rules" /etc/udev/rules.d/mango-rapl.rules

echo "==> reloading udev rules"
udevadm control --reload-rules

echo "==> applying to already-present powercap devices"
# ACTION=="add" only fires when a device is (re)created, so devices that were
# already there before this rule existed need one explicit trigger. Every
# reboot (or intel_rapl module reload) after this needs no such nudge.
udevadm trigger --action=add --subsystem-match=powercap

echo "==> verifying as $TARGET_USER"
if sudo -u "$TARGET_USER" cat /sys/class/powercap/intel-rapl:0/energy_uj > /dev/null 2>&1; then
	echo "    energy_uj is readable — RAPL unlocked"
else
	echo "    still unreadable as $TARGET_USER. Check:" >&2
	echo "      getent group wheel                                  # is $TARGET_USER a member?" >&2
	echo "      ls -l /sys/class/powercap/intel-rapl:0/energy_uj     # should be g+r, group wheel" >&2
	echo "      udevadm test -a add /sys/class/powercap/intel-rapl:0 # trace the rule" >&2
	exit 1
fi

echo "==> sudoers rule -> /etc/sudoers.d/mango-powertop"
visudo -c -q -f "$SRC_DIR/sudoers.d-mango-powertop"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-mango-powertop" /etc/sudoers.d/mango-powertop

cat <<EOF

Installed. Next:

  ironbar var get bat_tip                      # "Power" row should now show real watts
  sudo -n -l /usr/bin/powertop                 # must print the path with no password prompt
  sudo powertop --calibrate                    # once, on battery — teaches powertop the per-device power model

Rollback:
  sudo rm /etc/udev/rules.d/mango-rapl.rules /etc/sudoers.d/mango-powertop
  sudo udevadm control --reload-rules
  # energy_uj stays group-readable until the next boot or module reload; harmless either way
EOF
