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

## States

Written to `/run/mango-vpnguard/state` (world-readable, no root needed to
read it — `net.sh` and `mango-bard` poll this file directly):

| State | Egress policy | Default route | `mango-vpnguard status` says |
|---|---|---|---|
| `unconfigured` | untouched — no profile is opted in yet | whatever NM already has | not protected — no tunnel chosen yet |
| `blocked` | LAN + DHCP + every managed profile's dial endpoint | none | blocked — nothing can leave until a tunnel connects |
| `portal` | `blocked` plus DNS and tcp/80,443 to anywhere | uplink (temporary) | sign-in needed — captive portal on this network |
| `vpn:<name>` | out the winning connection's device, scoped to its `allowed-ips`; any always-on companion up too | that device | protected — `<name>` |
| `unsecured` | ufw restored to whatever it was before the first `arm` | uplink | not protected — traffic is leaving in the clear |

`unsecure` and `disarm` both land on `unsecured` — one remembers the network
so a later `auto` on it stays out of the way, the other does not. Either way,
`protect` is the way back: it forgets any opt-out and re-arms on the current
network, without waiting for a network change.

## Docker containers are covered too

`ufw`'s deny-outgoing only governs the host's own traffic. A running
container's egress is *forwarded*, and Docker inserts its own rules ahead of
ufw's in the kernel's forward chain — `blocked` would otherwise mean nothing
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
cloned or modified one (beyond `apply_routes`' scoped `ufw` rules, which
`disarm` already tears down), so there is nothing further to remove.
