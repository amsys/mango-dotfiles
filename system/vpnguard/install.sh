#!/usr/bin/env bash
# Installs the root-owned VPN-guard helper. Follows the system/hotspot/
# precedent exactly: sudo-gated, nothing under system/ is symlinked into
# ~/.config, so this script is the only way any of it lands.
#
#   sudo ~/src/mango-dotfiles/system/vpnguard/install.sh
#
# There is no vpnguard config file — NetworkManager is the source of truth.
# Choosing and ordering VPNs is a plain unprivileged `nmcli connection
# modify` after this script finishes, not an edit to a file here. See
# README.md.
# check: /usr/local/bin/mango-vpnguard
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
TARGET_USER="${SUDO_USER:-}"
[ -n "$TARGET_USER" ] || { echo "run with 'sudo', not as root directly — need \$SUDO_USER to verify the fix as" >&2; exit 1; }

for b in nft nmcli wg ip iptables; do
	command -v "$b" >/dev/null 2>&1 || { echo "missing dependency: $b — install it first" >&2; exit 1; }
done

echo "==> ruleset -> /usr/local/share/mango-vpnguard/vpn.nft"
nft -c -f "$SRC_DIR/vpn.nft" || { echo "vpn.nft does not parse — refusing to install a broken guard" >&2; exit 1; }
install -d -m 0755 -o root -g root /usr/local/share/mango-vpnguard
install -m 0644 -o root -g root "$SRC_DIR/vpn.nft" /usr/local/share/mango-vpnguard/vpn.nft

echo "==> route-table name -> /etc/iproute2/rt_tables.d/mango-vpnguard.conf"
install -d -m 0755 -o root -g root /etc/iproute2/rt_tables.d
install -m 0644 -o root -g root "$SRC_DIR/rt_tables.d-mango-vpnguard" \
	/etc/iproute2/rt_tables.d/mango-vpnguard.conf

echo "==> dnsmasq example -> /usr/local/share/mango-vpnguard/"
# Reference copy only. Nothing is placed in NetworkManager's dnsmasq.d: the
# domain-driven bypass is opt-in, and switching the machine's resolver is a
# decision, not an install step. README.md has the two commands.
install -m 0644 -o root -g root "$SRC_DIR/dnsmasq.d-mango-vpnguard.conf.example" \
	/usr/local/share/mango-vpnguard/dnsmasq.d-mango-vpnguard.conf.example

echo "==> helper -> /usr/local/bin/mango-vpnguard"
install -m 0755 -o root -g root "$SRC_DIR/mango-vpnguard" /usr/local/bin/mango-vpnguard

echo "==> self-check"
/usr/local/bin/mango-vpnguard test || { echo "mango-vpnguard test failed — stopping before touching the guard or sudoers" >&2; exit 1; }

echo "==> loader unit -> /etc/systemd/system/mango-vpnguard-nft.service"
install -m 0644 -o root -g root "$SRC_DIR/mango-vpnguard-nft.service" \
	/etc/systemd/system/mango-vpnguard-nft.service
systemctl daemon-reload
systemctl enable --now mango-vpnguard-nft.service
nft list table inet vpn >/dev/null 2>&1 || { echo "table inet vpn did not load" >&2; exit 1; }
echo "    table inet vpn loaded, every set empty (inert)"

echo "==> the escape hatch, before anything is armed:"
echo "      sudo mango-vpnguard disarm   # empties every set; nothing is denied"

echo "==> sudoers rule -> /etc/sudoers.d/mango-vpnguard"
visudo -c -q -f "$SRC_DIR/sudoers.d-mango-vpnguard"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-mango-vpnguard" /etc/sudoers.d/mango-vpnguard

echo "==> verifying as $TARGET_USER"
if sudo -n -u "$TARGET_USER" sudo -n /usr/local/bin/mango-vpnguard status </dev/null >/dev/null 2>&1; then
	echo "    mango-vpnguard status runs with no password prompt"
else
	echo "    still prompting for $TARGET_USER. Check:" >&2
	echo "      getent group wheel                     # is $TARGET_USER a member?" >&2
	echo "      sudo -l -U $TARGET_USER                 # should list the mango-vpnguard verbs" >&2
	exit 1
fi

echo "==> NM dispatcher -> /etc/NetworkManager/dispatcher.d/90-mango-vpnguard"
install -m 0755 -o root -g root "$SRC_DIR/dispatcher-90-mango-vpnguard" \
	/etc/NetworkManager/dispatcher.d/90-mango-vpnguard

cat <<EOF

Installed. Nothing is armed yet — every WireGuard profile still has
autoconnect-priority=0, so the dispatcher's next link event writes state
'unconfigured' and leaves networking alone (mango-vpnguard never blocks
outgoing traffic until you pick something). The nftables table is loaded but
every set in it is empty, which is the same as not being there at all.

Choose the default-route chain and any always-on companion with plain nmcli
— no root, no config file, no re-running this script:

  nmcli connection modify "<name>" connection.autoconnect-priority 100   # highest tried first
  nmcli connection modify "<name>" connection.autoconnect yes            # always-on companion

Once at least one profile is set:

  sudo mango-vpnguard arm
  sudo mango-vpnguard auto   # arm, then dial the chain in priority order

'arm' warns about any managed profile whose connection.permissions is
non-empty — that profile is restricted to one user's session and will not
dial before login (the NM dispatcher's boot-time 'auto' hits the same
restriction). Clear it if you want that VPN up before you log in:

  nmcli connection modify "<name>" connection.permissions ""

Check state or the managed VPNs any time (no root needed, world-readable):

  cat /run/mango-vpnguard/state
  mango-vpnguard list

Rollback:

  sudo mango-vpnguard disarm
  sudo systemctl disable --now mango-vpnguard-nft.service
  sudo rm -r /usr/local/bin/mango-vpnguard /etc/sudoers.d/mango-vpnguard \\
          /etc/systemd/system/mango-vpnguard-nft.service \\
          /usr/local/share/mango-vpnguard \\
          /etc/iproute2/rt_tables.d/mango-vpnguard.conf \\
          /etc/NetworkManager/dispatcher.d/90-mango-vpnguard

'arm' also builds the 'direct' route table, which is what a bypass leaves
by. It stays inert until something fills the 'bypass4' set — see "Bypassing
the tunnel by domain" in README.md, which is opt-in and switches the
machine's resolver to dnsmasq.

See vpnguard/README.md for the state table and the portal exposure window.
EOF
