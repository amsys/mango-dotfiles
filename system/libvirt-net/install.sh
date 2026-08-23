#!/usr/bin/env bash
# Lets libvirt's NAT network (virbr0, network "default") reach VM guests
# through ufw. Follows the system/hotspot/ precedent: sudo-gated, nothing
# under system/ is symlinked into ~/.config, so this script is the only way
# the fix lands.
#
#   sudo ~/src/mango-dotfiles/system/libvirt-net/install.sh
#
# See README.md in this directory for why ufw blocks libvirt DHCP/DNS by
# default even though libvirt's own nftables rules look correct.
set -eu

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }

command -v ufw >/dev/null 2>&1 || { echo "missing dependency: ufw" >&2; exit 1; }
ip link show virbr0 >/dev/null 2>&1 || {
	echo "virbr0 not found — start the libvirt 'default' network first:" >&2
	echo "  virsh net-start default && virsh net-autostart default" >&2
	exit 1
}

echo "==> allowing libvirt DHCP/DNS through ufw on virbr0"
ufw allow in on virbr0 to any port 67 proto udp comment 'libvirt DHCP'
ufw allow in on virbr0 to any port 53 comment 'libvirt DNS'

echo "==> checking forward policy"
if ufw status verbose | grep -q '^Default: .*allow (routed)'; then
	echo "    already allow (routed) — nothing to add"
else
	echo "    routed traffic is not allowed by default — scoping a rule to virbr0"
	ufw route allow in on virbr0 comment 'libvirt VM egress'
fi

cat <<EOF

Done. In the VM: ipconfig /release && ipconfig /renew (Windows) or
dhclient (Linux) to pick up a lease immediately.

If a full-tunnel VPN (e.g. NordVPN) is later found to break VM networking,
allowlist the libvirt subnet instead of touching these ufw rules:
  sudo nordvpn allowlist add subnet 192.168.122.0/24

Rollback:
  sudo ufw delete allow in on virbr0 to any port 67 proto udp
  sudo ufw delete allow in on virbr0 to any port 53
  sudo ufw route delete allow in on virbr0   # only if this script added it
EOF
