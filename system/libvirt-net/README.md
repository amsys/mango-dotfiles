# libvirt NAT network vs ufw

## Symptom

A VM on libvirt's `default` NAT network (bridge `virbr0`) gets no DHCP lease
and no network, even though `virsh net-dumpxml default` looks correct and
`ip_forward` is on.

## Cause

ufw's default input policy is REJECT. The VM's DHCP request (udp/67) and DNS
query (53) to the bridge address (`192.168.122.1`) are host **input**
packets, so ufw rejects them before dnsmasq ever answers.

libvirt writes its own `accept` rules into an `nft` table
(`libvirt_network`), but that does not help: in nftables, an `accept`
verdict only ends the current table's chain. The packet still crosses ufw's
base chain at the same hook, where the REJECT policy is final. So libvirt
"looks configured correctly" while nothing works.

The blocked packets do not show up in `journalctl -k | grep 'UFW BLOCK'`:
`/etc/ufw/after.rules` silences dport 67 logging by default, which makes
this look like a routing problem instead of a firewall one.

## Fix

Two `ufw allow in on virbr0` rules, scoped to ports 67 and 53 — not a
wholesale `allow in on virbr0`, because this bridge carries an untrusted
Windows VM and the host also listens on ssh, a custom port, and
Postgres/Redis for docker. See `install.sh`.

Rules key on the **inbound interface** (`virbr0`), never on an egress
interface or a subnet outside 192.168.122.0/24. `virbr0` is created once by
libvirt and never changes; the laptop's wifi interface, IP, and any active
VPN interface change constantly (roaming, suspend/resume, VPN
connect/disconnect). libvirt's own masquerade rule is source-based
(`ip saddr 192.168.122.0/24`), so NAT already follows whatever the current
default route is — nothing here needs to react to those changes. ufw
persists rules in `/etc/ufw/user.rules`, so they also survive reboot.

## VPN interaction

VM traffic is **not** tunnelled through NordVPN by design: NordLynx routes
by fwmark, which forwarded (VM) packets never carry, so the VM always exits
through the host's normal uplink even when the host itself is tunnelled.
This is accepted, not a bug.

If a full-tunnel VPN's own firewall/kill switch ever blocks the VM (not
observed on the NordVPN NORDLYNX default at time of writing), the fix is
`nordvpn allowlist add subnet 192.168.122.0/24` — not a change to these ufw
rules.
