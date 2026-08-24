#!/usr/bin/env bash
# Opens the remote-access bar toggle's ports (wayvnc 5900, KDE Connect
# 1714-1764) through ufw, scoped to wg_hetzner only. Follows the
# system/libvirt-net/ precedent: sudo-gated, nothing under system/ is
# symlinked into ~/.config, so this script is the only way the rule lands.
#
#   sudo ~/src/mango-dotfiles/system/remote/install.sh
#
# See README.md in this directory for why wg_hetzner (not a source IP) is
# the right scope, and why the hotspot needs no rule here at all.
set -eu

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }

command -v ufw >/dev/null 2>&1 || { echo "missing dependency: ufw" >&2; exit 1; }
ip link show wg_hetzner >/dev/null 2>&1 || {
	echo "wg_hetzner not found — bring the tunnel up first (nmcli con up 'Hetzner Gateway')" >&2
	exit 1
}

echo "==> allowing wayvnc + kdeconnectd through ufw on wg_hetzner"
ufw allow in on wg_hetzner to any port 5900 proto tcp comment 'vnc via hetzner wg'
ufw allow in on wg_hetzner to any port 1714:1764 proto tcp comment 'kdeconnect via hetzner wg'
ufw allow in on wg_hetzner to any port 1714:1764 proto udp comment 'kdeconnect via hetzner wg'

cat <<EOF

Done. The mango hotspot needs no rule here — system/hotspot/mango-hotspot
already opens a blanket 'ufw allow in on p2p0' while it is up.

wayvnc binds 0.0.0.0 (v4 only); reaching it over the tunnel's IPv6 address
(fd10:10::beef) would need wayvnc started with :: instead.

Rollback:
  sudo ufw delete allow in on wg_hetzner to any port 5900 proto tcp
  sudo ufw delete allow in on wg_hetzner to any port 1714:1764 proto tcp
  sudo ufw delete allow in on wg_hetzner to any port 1714:1764 proto udp
EOF
