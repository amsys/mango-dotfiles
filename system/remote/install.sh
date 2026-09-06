#!/usr/bin/env bash
# Opens the remote-access bar toggle's ports (wayvnc 5900, KDE Connect
# 1714-1764) through ufw, scoped by interface AND source subnet. wayvnc runs
# with no auth and no encryption (README.md's "Scope" section) — these rules
# are the only boundary, not defense in depth on top of one.
# Follows the system/libvirt-net/ precedent: sudo-gated, nothing under
# system/ is symlinked into ~/.config, so this script is the only way the
# rule lands.
#
#   sudo ~/src/mango-dotfiles/system/remote/install.sh
#
# Two sources are allowed: wg_hetzner (the WireGuard tunnel,
# 10.0.254.0/24 + fd10:10::/64) and the mango hotspot (p2p0,
# 10.44.0.0/24, IPv4 only — it hands out no IPv6). Both device and
# subnets are hardcoded; override the devices with MANGO_REMOTE_WG_DEV /
# MANGO_HS_IFACE if this machine names them differently. See README.md in
# this directory for the trust model.
# check: ufw:vnc via
set -eu

WG_DEV="${MANGO_REMOTE_WG_DEV:-wg_hetzner}"
WG_NET4="10.0.254.0/24"
WG_NET6="fd10:10::/64"

HS_DEV="${MANGO_HS_IFACE:-p2p0}"
HS_NET4="10.44.0.0/24"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }

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
EOF
