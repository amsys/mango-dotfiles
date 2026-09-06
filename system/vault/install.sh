#!/usr/bin/env bash
# Grant the active-seat user an ACL on the Power Button evdev node, so
# mango-powerkey.service keeps working after `martin` leaves the `input`
# group. Follows the system/rapl/ precedent: sudo-gated, and nothing under
# system/ is symlinked into ~/.config, so this script is the only way it lands.
#
#   sudo ~/src/mango-dotfiles/system/vault/install.sh
#
# Part of the wallet-isolation + input-hardening design in VAULT.md. See
# 72-vault-powerbtn.rules for why a per-node uaccess rule is used, and
# VAULT.md section 1 for the full sniffing threat model.

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER to verify the ACL as" >&2; exit 1; }

# Find the Power Button evdev node by name, not by event number (the number is
# not stable across boots; powerkey.py itself matches on the name).
PB=""
for d in /sys/class/input/event*; do
	[ "$(cat "$d/device/name" 2>/dev/null)" = "Power Button" ] || continue
	PB="/dev/input/${d##*/}"; break
done
[ -n "$PB" ] || { echo "no evdev node named 'Power Button' found — cannot verify" >&2; exit 1; }
echo "==> Power Button node: $PB"

echo "==> udev rule -> /etc/udev/rules.d/72-vault-powerbtn.rules"
install -m 0644 -o root -g root "$SRC_DIR/72-vault-powerbtn.rules" \
	/etc/udev/rules.d/72-vault-powerbtn.rules

echo "==> reloading udev rules"
udevadm control --reload-rules

echo "==> applying to the already-present Power Button node"
# The tag is acted on when the device is (re)created; a node present before the
# rule existed needs one explicit change trigger. Reboots after this do not.
udevadm trigger --subsystem-match=input --action=change
udevadm settle

echo "==> verifying the ACL as $TARGET_USER"
if getfacl -p "$PB" 2>/dev/null | grep -q "^user:$TARGET_USER:.*r"; then
	echo "    $PB is readable by $TARGET_USER — power button will survive the input-group drop"
else
	echo "    no ACL for $TARGET_USER on $PB. Check:" >&2
	echo "      loginctl show-session \$(loginctl | awk -v u=$TARGET_USER '\$3==u{print \$1;exit}') -p Active -p Seat  # must be Active=yes on a seat" >&2
	echo "      udevadm test /sys/class/input/${PB##*/} 2>&1 | grep -i uaccess          # trace the rule" >&2
	exit 1
fi

cat <<TXT

Installed. This grants back ONLY the power button; while $TARGET_USER is still
in the 'input' group the ACL is redundant. It becomes the sole access path
after you drop the group:

  sudo gpasswd -d $TARGET_USER input     # effective at next login

Then, after a fresh login, confirm the hardening actually took:

  fuser -v /dev/input/event*             # only mango should hold evdev
  getfacl -p $PB                         # user:$TARGET_USER:rw- present
  getfacl -p /dev/input/event1           # a keyboard: NO user:$TARGET_USER line

Rollback:
  sudo rm /etc/udev/rules.d/72-vault-powerbtn.rules
  sudo udevadm control --reload-rules
  sudo gpasswd -a $TARGET_USER input     # if you had dropped it; relogin to apply
TXT
