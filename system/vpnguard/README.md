# vpnguard

Fail-closed networking: deny egress by default, dial a WireGuard tunnel
automatically, fail over to the next one by priority if the first will not
come up, and let the user decide about "unsecured" only when every automatic
option has failed.

NetworkManager is the only source of truth. There is no vpnguard config
file. Every WireGuard connection NM already knows about is a candidate —
opting one in, ordering the chain, or making one an always-on companion is a
plain, unprivileged `nmcli connection modify`. vpnguard never creates,
clones or imports a tunnel profile; that stays yours (or
[`nordlynx-import`](#nordlynx-import--nordvpn-servers-as-nm-profiles)'s, for
a NordVPN server).

Install:

```
sudo ~/src/mango-dotfiles/system/vpnguard/install.sh
```

Nothing is armed by a fresh install: every WireGuard profile starts at
`autoconnect-priority=0`, so the guard has nothing to dial and stays out of
the way. Pick your chain afterwards:

```
nmcli connection modify "<name>" connection.autoconnect-priority 100   # highest tried first
nmcli connection modify "<name>" connection.autoconnect yes            # always-on companion
```

Run `mango-vpnguard help` for the verb list. See `../hotspot/` /
`../powermode/` for the pattern this follows (root helper in
`/usr/local/bin`, fixed sudoers verbs, a `test` subcommand that needs no
root).

## The NM-native schema

| vpnguard reads | from |
|---|---|
| identity and label | the connection name, verbatim (may contain spaces, parens) |
| chain membership and order | `connection.autoconnect-priority` — any value > 0, dialled highest first |
| always-on companion | `connection.autoconnect` = `yes` |
| device | `connection.interface-name` |
| dial endpoint (the port opened while blocked) | `wireguard.peers[0].endpoint` |
| routes, and which IP family this peer carries | `wireguard.peers[0].allowed-ips` |

Only `wireguard`-type connections are eligible. `autoconnect-priority` does
nothing to NM's own activation when a profile's `autoconnect` is `no` (the
usual case for a chain entry — you dial it through vpnguard, not NM's own
autoconnect), so vpnguard reading it as chain order never fights NM.

Endpoint rotation — trying the same peer on a second port when the first is
filtered — is one NM profile per port, all opted into the chain at
descending priority. That is the same rotation the old config-file version
did with `ENDPOINTS="host:51820 host:443 ..."`; it is now expressed as
ordinary profiles instead of a second concept.

## Roles

- **always-on companion** (`connection.autoconnect=yes`) — brought up after
  the default-route winner is decided, and skipped if it shares that
  winner's device (two profiles cannot own one interface). This is the
  split-tunnel case: a gateway's internal prefixes stay reachable no matter
  which full tunnel is active.
- **chain entry** (`connection.autoconnect-priority>0`) — competes for the
  default route. `mango-vpnguard auto` walks the chain highest-priority
  first and takes the first one that produces a real WireGuard handshake,
  checked with `wg show <dev> latest-handshakes` — NM reports "activated"
  for a link that has never handshaked, so its exit code is not trusted.

## Enforcement is one nftables table

The guard is `table inet vpn`, loaded from
`/usr/local/share/mango-vpnguard/vpn.nft` by
`mango-vpnguard-nft.service` before any interface comes up. The file holds
no address, no interface name and no site value: every value is a named set
that this helper fills at run time.

`uplinks` is the arm switch. Both base chains accept a packet at once when
its interface is not in that set, so an empty `uplinks` is an open machine
and a filled one is a closed machine. Arming, portal mode and every timed
opening add or remove set elements — the ruleset itself is never reloaded
after boot.

Both base chains use `policy drop`, so a partial load blocks rather than
opens. `arm` calls the loader itself and refuses to arm when the table
cannot be loaded; the boot unit deliberately does not gate the network,
because trading a firewall bug for an unreachable laptop is the worse
failure.

The table is its own. It never reads or writes another firewall's rules and
no other firewall reads or writes it. Netfilter runs every base chain on a
hook and one drop is final, so a packet must pass both. `ufw` is therefore
irrelevant to the guard, whether it is enabled or not.

Order matters inside `arm`: every hole is punched first and `uplinks` is
written last. Arming first would close the machine for as long as it takes
to write the endpoint holes, and that window kills the tunnel being dialled.

## Policy routing: the `direct` lane

Some traffic has to leave in the clear even while the guard is closed — a
speed test, a video-call SFU that a VPN address gets throttled on. That is a
routing decision, not a firewall decision, so it takes two halves.

The ruleset's `marks` chain sets mark `0x1` on packets to any address in
`bypass4`. It is a `route` chain, not a filter chain, and that is the whole
point: at filter priority the kernel has already chosen the route, while a
`route` chain re-runs the route lookup when the mark changes.

`arm` supplies the other half — route table `direct`, holding one default via
the physical uplink's own gateway, and the `ip rule` that sends marked packets
to it. `disarm` removes both.

Two consequences worth knowing:

- `arm` reads that gateway **while the uplink's own default route is still the
  machine's default route**, before any tunnel is dialled. It re-reads it on
  every uplink change, so the table can never hold a gateway from the last
  network.
- If the uplink has no gateway to fall back to, `arm` says so and installs no
  rule at all. A marked packet then keeps the route it already had — the
  tunnel. The bypass silently stops working, and nothing leaks. That is the
  right way round to fail, and it is the way it fails whenever anything in
  this lane goes wrong.

The table id and rule priority are fixed numbers with no site meaning.
`rt_tables.d-mango-vpnguard` only gives the id a readable name; the helper
addresses the table by number, so a missing name cannot break anything.

## Bypassing the tunnel by domain

`bypass4` starts empty and nothing in this repo fills it. Filling it by hand
works (`nft add element inet vpn bypass4 { 203.0.113.5 }`), but the case that
matters is a domain, not an address: a site behind a CDN rotates its addresses
faster than any refresh interval, so a list resolved on a timer is a list that
is wrong most of the time. The set has to be filled per DNS answer.

dnsmasq does that natively with `nftset=`. systemd-resolved has no equivalent,
so this is opt-in and it switches the machine's resolver:

```
# 1. Point NetworkManager at dnsmasq. Change the `dns=` line that is already
#    there. Do NOT add a new drop-in: conf.d files merge in alphabetical
#    order and the LAST one wins, so a new file whose name sorts earlier than
#    the existing one is read and then silently overridden. Find the file:
grep -rn '^dns=' /etc/NetworkManager/
#    set that line to `dns=dnsmasq`, then confirm NM's merged view agrees:
NetworkManager --print-config | sed -n '/^\[main\]/,/^\[/p' | grep '^dns='

# 2. systemd-resolved has to stop owning /etc/resolv.conf first. NM then
#    writes its own (rc-manager=symlink) — but only if nothing is in the way.
sudo systemctl disable --now systemd-resolved
sudo rm /etc/resolv.conf

# 3. Your domains.
sudo install -m 0644 /usr/local/share/mango-vpnguard/dnsmasq.d-mango-vpnguard.conf.example \
     /etc/NetworkManager/dnsmasq.d/mango-vpnguard.conf
sudoedit /etc/NetworkManager/dnsmasq.d/mango-vpnguard.conf

# 4. Swap the plugin in place. This does NOT restart NetworkManager and does
#    NOT drop the Wi-Fi association, which matters on someone else's network.
sudo nmcli general reload conf
sudo nmcli general reload dns-full
```

Steps 1, 3 and 4 change nothing about the system resolver: dnsmasq comes up
on 127.0.0.1 while whatever owns `/etc/resolv.conf` keeps owning it. So the
whole chain can be proved before committing to anything —
`dig @127.0.0.1 <a listed domain>`, then
`sudo nft list set inet vpn bypass4` to see the answer land in the set. Do
step 2 only once that works.

NetworkManager keeps generating per-connection DNS under `dns=dnsmasq`,
including split DNS for a gateway's private domains, so that stays NM's job
and stays out of this repo.

Three things to check afterwards, because each fails quietly in its own way:

- `ps -o user,args -C dnsmasq` — NetworkManager starts dnsmasq as root and it
  drops to `nobody`, keeping only `CAP_NET_ADMIN`, which is what writing an
  nftables set needs. dnsmasq asks for that capability only when a `nftset=`
  line is present, and refuses to start without it rather than failing
  silently: `process is missing required capability CAP_NET_ADMIN`.
- The table must exist before dnsmasq starts, or every answer logs an error.
  `mango-vpnguard-nft.service` runs before `network-pre.target`, so it is
  already ordered ahead of NetworkManager.
- Split DNS moves with the plugin, but NM pushes it over D-Bus rather than
  writing it to a file, so there is nothing to read in `dnsmasq.d`. Confirm
  it in the journal instead — `journalctl -u NetworkManager | grep "using
  nameserver"` prints one unambiguous line per routed domain.
- `resolvectl` is gone with resolved. Read DNS state with
  `nmcli device show <dev> | grep IP4.DNS` instead.

Rollback, in this order:

```
sudo sed -i 's/^dns=dnsmasq/dns=systemd-resolved/' <the file from step 1>
sudo systemctl enable --now systemd-resolved
sudo ln -sf ../run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
sudo systemctl restart NetworkManager
```

The symlink has to go back by hand. systemd ships it as `L!` in
`tmpfiles.d/systemd-resolve.conf`, and `!` means boot-only — a runtime
`systemd-tmpfiles --create` will not recreate it.

## States

Written to `/run/mango-vpnguard/state` (world-readable, no root needed to
read it — `net.sh` and `mango-bard` poll this file directly):

| State | Egress policy | Default route | `mango-vpnguard status` says |
|---|---|---|---|
| `unconfigured` | untouched — no profile is opted in yet | whatever NM already has | not protected — no tunnel chosen yet |
| `blocked` | LAN + DHCP + every managed profile's dial endpoint | none | blocked — nothing can leave until a tunnel connects |
| `portal` | `blocked` plus DNS and tcp/80,443 to anywhere | uplink (temporary) | sign-in needed — captive portal on this network |
| `vpn:<name>` | out the winning connection's device, scoped to its `allowed-ips`; any always-on companion up too | that device | protected — `<name>` |
| `unsecured` | guard open — every set empty, nothing denied | uplink | not protected — traffic is leaving in the clear |

`unsecure` and `disarm` both land on `unsecured` — one remembers the network
so a later `auto` on it stays out of the way, the other does not. Either way,
`protect` is the way back: it forgets any opt-out and re-arms on the current
network, without waiting for a network change.

## Docker containers are covered too

The guard's output chain only governs the host's own traffic. A running
container's egress is *forwarded*, and Docker inserts its own rules ahead of
the guard's in the kernel's forward chain — `blocked` would otherwise mean nothing
to `docker run`. `arm` adds a private `MANGO-VPNGUARD` chain jumped from
`DOCKER-USER` (both `iptables` and `ip6tables`), dropping container egress
out the physical uplink; once a tunnel wins, a `RETURN` rule for that
tunnel's device is inserted ahead of the drop. `disarm` deletes the chain and
its jump — Docker's own rules are never touched.

## Generated sudoers

`sudoers.d-mango-vpnguard` is a static, tracked file (unlike the old
per-config-id generation) — every managed profile is looked up live, so
there is no id list to keep in sync. `up` is the one verb with a payload:
an NM connection name can contain spaces and parens, so it cannot be a
sudoers literal. It arrives as `CONN=<name>` on stdin — sudoers cannot see
or restrict that, so `mango-vpnguard` re-validates it against the live
chain (a closed set derived from NM at call time) before dialling anything.
Same idiom as `sudoers.d-mango-powermode`'s stdin payload.

## Why the dispatcher hands off instead of dialling inline

NetworkManager kills a dispatcher script that runs past 90 seconds. With
several chain entries each a possible endpoint, walking the whole chain
inline can take longer than that. `dispatcher-90-mango-vpnguard` hands the
dial to `systemd-run --no-block`, so it returns at once and the dial runs
outside the watchdog entirely. `MANGO_VG_BUDGET` (default 90s) is a backstop
inside `mango-vpnguard auto` itself, not the primary mechanism — it stops a
pathological chain (far more entries than anyone would configure) from
dialling forever.

## HTTPS captive portals

A portal cannot intercept HTTPS: TLS interception fails certificate
validation. An "HTTPS portal" intercepts over plain HTTP or DNS and serves
only its *login page* over HTTPS, on its own hostname.

Two rules follow. Detection must use plain HTTP — an HTTPS probe against a
portal returns a TLS error, which cannot be told apart from network-down,
DPI-blocking or server-down, while an HTTP probe returns an unambiguous 302
against 204. Login must have 443 open as well as 80, plus DNS, because the
portal's own hostname usually resolves only through the portal's resolver.
`portal` state opens exactly those three.

A portal that hijacks DNS and does not redirect HTTP shows up as
connectivity `limited` or `none`, not `portal`. That case cannot be
detected; it needs `portal` by hand.

NetworkManager requests the RFC 8910 captive-portal URI (DHCP option 114)
and hands it to the dispatcher as `DHCP4_CAPTIVE_PORTAL` when the network
sends it. Most networks do not.

### The probe needs a hole, and the hole follows NM

A closed guard blocks NM's own connectivity check too, and NM can then never
report `portal`. So `arm` opens tcp/80 and tcp/443 to the probe host, and it
reads that host from NM itself — `NetworkManager --print-config`, the merged
view of every `conf.d` drop-in, section `[connectivity]`, key `uri`. No probe
address is written in this repo: whichever host you configure is the host
that gets the hole.

Point that URI at a host you own. A third-party probe host is a hole in the
guard to a machine you do not control, and it tells that machine every
network you join.

The hole is resolved at arm time, while the guard is still open, because a
closed guard has no DNS left to resolve it with. `arm` says so and continues
when there is no usable probe: the URI is unset, its host does not resolve to
IPv4, it is an IPv6 literal, or it carries a port other than 80 or 443. The
guard still protects the machine; only portal detection is lost.

## The portal window is real exposure

`portal` relaxes DNS and ports 80/443 to anywhere, on the physical uplink,
before any tunnel exists. That is a deliberate trade: a captive portal you
cannot reach is a portal you cannot sign in to. It only opens when NM
reports connectivity `portal`/`limited`, and it closes the moment
connectivity reaches `full` — at which point the dial starts. There is no
way to get hotel/airport Wi-Fi working without this window; it is the
minimum, not a shortcut.

## Per-uplink unsecured opt-out

Pressing "Go unprotected" on the failure notification remembers the
network as `SSID@BSSID` (or `wired@<dev>`) in
`/run/mango-vpnguard/unsecured-key` — not persisted to disk, gone at
reboot. Rejoining the *same* association skips re-arming; a *different* one
(even the same SSID at a different location — different BSSID) re-arms and
asks again.

## A profile restricted to your login session will not dial at boot

`arm` warns when a managed profile's `connection.permissions` is non-empty
— NM will not activate that profile for the pre-login dispatcher run (or
for any root dial). Clear it if you want that VPN up before you log in:

```
nmcli connection modify "<name>" connection.permissions ""
```

## `nordlynx-import` — NordVPN servers as NM profiles

NordVPN publishes no WireGuard config files. `nordlynx-import` (unprivileged,
standalone, never touches vpnguard) builds one from NordVPN's own API and
hands it to `nmcli connection import` — after that, vpnguard treats it like
any other WireGuard profile, and stays entirely Nord-agnostic.

```
~/src/mango-dotfiles/system/vpnguard/nordlynx-import <server-hostname>
```

The server list and each server's public key need no authentication. The
private key needs your account's access token (from NordVPN's manual
configuration page), read from `~/.config/mango/nordvpn-token` (mode 0600,
never read by root) and never written to disk outside the moment the `.conf`
is imported — see the script's own header for the exact flow.

## Country-specific egress

Out of scope here by design: once a full tunnel is the default route, the
VPN endpoint itself decides which prefixes exit where. Nothing on the
laptop needs to know about it.

## Rollback

```
sudo mango-vpnguard disarm
sudo rm /usr/local/bin/mango-vpnguard /etc/sudoers.d/mango-vpnguard \
        /etc/NetworkManager/dispatcher.d/90-mango-vpnguard
```

Every WireGuard profile stays exactly as it was — vpnguard never created,
cloned or modified one, so there is nothing further to remove.
