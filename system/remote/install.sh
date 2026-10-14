#!/usr/bin/env bash
# Opens the remote-access bar toggle's ports (wayvnc 5900, KDE Connect
# 1714-1764) through ufw, scoped by interface AND source subnet, and
# installs the root physical-lock helper (SPEC.md §7.2-7.3): the
# mango-physlock binary, its system unit and its sudoers fragment. wayvnc
# runs with no auth and no encryption (README.md's "Scope" section) —
# the ufw rules are the only boundary for that half, not defense in depth
# on top of one. Follows the system/libvirt-net/ precedent for the ufw
# half and the system/powermode/ precedent (helper + visudo -cf-checked
# sudoers fragment) for the physlock half: sudo-gated, nothing under
# system/ is symlinked into ~/.config, so this script is the only way
# either lands.
#
#   sudo ~/src/mango-dotfiles/system/remote/install.sh
#
# Two sources are allowed through ufw: wg_hetzner (the WireGuard tunnel,
# 10.0.254.0/24 + fd10:10::/64) and the mango hotspot (p2p0,
# 10.44.0.0/24, IPv4 only — it hands out no IPv6). Both device and
# subnets are hardcoded; override the devices with MANGO_REMOTE_WG_DEV /
# MANGO_HS_IFACE if this machine names them differently. See README.md in
# this directory for the trust model.
# check: ufw:vnc via
# check: /usr/local/bin/mango-physlock
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

WG_DEV="${MANGO_REMOTE_WG_DEV:-wg_hetzner}"
WG_NET4="10.0.254.0/24"
WG_NET6="fd10:10::/64"

HS_DEV="${MANGO_HS_IFACE:-p2p0}"
HS_NET4="10.44.0.0/24"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER for the physlock verify step" >&2; exit 1; }

command -v ufw >/dev/null 2>&1 || { echo "missing dependency: ufw" >&2; exit 1; }
ip link show "$WG_DEV" >/dev/null 2>&1 || {
	echo "$WG_DEV not found — bring that tunnel up first, or set MANGO_REMOTE_WG_DEV" >&2
	exit 1
}
# No such guard for $HS_DEV: p2p0 only exists while the hotspot is up. A ufw
# rule naming an absent interface is inert, not an error, so it is safe to
# add before the hotspot's first start.

echo "==> allowing wayvnc + kdeconnectd through ufw on $WG_DEV ($WG_NET4, $WG_NET6)"
ufw allow in on "$WG_DEV" from "$WG_NET4" to any port 5900 proto tcp comment "vnc via $WG_DEV"
ufw allow in on "$WG_DEV" from "$WG_NET6" to any port 5900 proto tcp comment "vnc v6 via $WG_DEV"
ufw allow in on "$WG_DEV" from "$WG_NET4" to any port 1714:1764 proto tcp comment "kdeconnect via $WG_DEV"
ufw allow in on "$WG_DEV" from "$WG_NET4" to any port 1714:1764 proto udp comment "kdeconnect via $WG_DEV"
ufw allow in on "$WG_DEV" from "$WG_NET6" to any port 1714:1764 proto tcp comment "kdeconnect v6 via $WG_DEV"
ufw allow in on "$WG_DEV" from "$WG_NET6" to any port 1714:1764 proto udp comment "kdeconnect v6 via $WG_DEV"

echo "==> allowing wayvnc + kdeconnectd through ufw on $HS_DEV ($HS_NET4)"
ufw allow in on "$HS_DEV" from "$HS_NET4" to any port 5900 proto tcp comment "vnc via hotspot"
ufw allow in on "$HS_DEV" from "$HS_NET4" to any port 1714:1764 proto tcp comment "kdeconnect via hotspot"
ufw allow in on "$HS_DEV" from "$HS_NET4" to any port 1714:1764 proto udp comment "kdeconnect via hotspot"

echo "==> physlock helper -> /usr/local/bin/mango-physlock"
install -m 0755 -o root -g root "$SRC_DIR/physlock.py" /usr/local/bin/mango-physlock

echo "==> self-check"
/usr/local/bin/mango-physlock --selftest || {
	echo "mango-physlock --selftest failed — not installing its unit or sudoers rule" >&2
	exit 1
}

echo "==> system unit -> /etc/systemd/system/mango-physlock.service"
install -m 0644 -o root -g root "$SRC_DIR/mango-physlock.service" \
	/etc/systemd/system/mango-physlock.service
systemctl daemon-reload
# Not enabled: started and stopped on demand by remote.sh --privacy-watch
# only, same "nothing runs until the user session asks for it" policy as
# wayvnc.service — see mango-physlock.service's own header.

echo "==> sudoers rule -> /etc/sudoers.d/mango-physlock"
visudo -c -q -f "$SRC_DIR/mango-physlock.sudoers"
install -m 0440 -o root -g root "$SRC_DIR/mango-physlock.sudoers" /etc/sudoers.d/mango-physlock

echo "==> verifying the physlock sudoers rule as $TARGET_USER"
# Checked via `sudo -l`, not by actually starting the service: unlike
# mango-powermode/mango-hotspot's apply-for-real verification, starting
# mango-physlock for real grabs every physical keyboard/touchpad/mouse on
# this machine (that is its whole job) — not something an install script
# should risk doing to input on a box nobody asked it to lock down yet.
RULES="$(sudo -l -U "$TARGET_USER" 2>/dev/null || true)"
if echo "$RULES" | grep -q "systemctl start mango-physlock\.service" \
	&& echo "$RULES" | grep -q "systemctl stop mango-physlock\.service"; then
	echo "    $TARGET_USER can run both systemctl verbs with no password prompt"
else
	echo "    sudo -l -U $TARGET_USER does not list both mango-physlock verbs. Check:" >&2
	echo "      getent group wheel     # is $TARGET_USER a member?" >&2
	echo "      visudo -c              # re-check every sudoers.d fragment for a conflict" >&2
	exit 1
fi

cat <<EOF

Done.

Rollback:
  sudo ufw delete allow in on $WG_DEV from $WG_NET4 to any port 5900 proto tcp
  sudo ufw delete allow in on $WG_DEV from $WG_NET6 to any port 5900 proto tcp
  sudo ufw delete allow in on $WG_DEV from $WG_NET4 to any port 1714:1764 proto tcp
  sudo ufw delete allow in on $WG_DEV from $WG_NET4 to any port 1714:1764 proto udp
  sudo ufw delete allow in on $WG_DEV from $WG_NET6 to any port 1714:1764 proto tcp
  sudo ufw delete allow in on $WG_DEV from $WG_NET6 to any port 1714:1764 proto udp
  sudo ufw delete allow in on $HS_DEV from $HS_NET4 to any port 5900 proto tcp
  sudo ufw delete allow in on $HS_DEV from $HS_NET4 to any port 1714:1764 proto tcp
  sudo ufw delete allow in on $HS_DEV from $HS_NET4 to any port 1714:1764 proto udp
  sudo systemctl stop mango-physlock.service 2>/dev/null
  sudo rm /etc/sudoers.d/mango-physlock /etc/systemd/system/mango-physlock.service /usr/local/bin/mango-physlock
  sudo systemctl daemon-reload
EOF
