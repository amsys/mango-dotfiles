# Remote access: wayvnc + KDE Connect

## What this is

One ironbar pill (`remote`, right end of the bar) toggles two services
together:

- **wayvnc** — VNC server, the screen. Talks to mango directly (wlr-screencopy
  + virtual pointer/keyboard), no portal involved.
- **kdeconnectd** — KDE Connect daemon, input/clipboard/files/notifications
  from a paired phone. Has no screen view of its own.

Neither is enabled at login — `install-config.sh` links both units but does
not `systemctl --user enable` them (see `LINK_ONLY_UNITS` there). The pill's
click handler (`ironbar/scripts/remote.sh --toggle`) is the only thing that
starts or stops them.

## Why not krdp

krdp (KDE's RDP server) needs `org.freedesktop.portal.RemoteDesktop`. The
active portal backend on mango is `xdg-desktop-portal-wlr`
(`/usr/share/xdg-desktop-portal/mango-portals.conf` routes ScreenCast and
Screenshot to it), which does not implement RemoteDesktop. The only
installed backend that does is `kde.portal`, gated `UseIn=KDE`. wayvnc
sidesteps the portal stack entirely, which is why it works here today.

## Scope: wg_hetzner + hotspot, ufw as the only boundary

wayvnc runs with no auth and no encryption (no `~/.config/wayvnc/config` —
`enable_auth` defaults to `false`), and kdeconnectd has no auth of its own
either way. Both daemons bind `0.0.0.0` and `::` — neither has a
bind-address option that can be scoped to one interface's real address, so
**ufw is the only boundary**, not the bind address. `install.sh` in this
directory adds source-and-interface-scoped rules for both paths that are
allowed to reach these ports:

- `wg_hetzner` (the WireGuard tunnel), sources `10.0.254.0/24` and
  `fd10:10::/64`
- the mango hotspot's `p2p0`, source `10.44.0.0/24` (IPv4 only — the
  hotspot hands out no IPv6)

WireGuard's cryptokey routing (only an authenticated peer's traffic ever
emerges from `wg_hetzner`) is what authenticates the tunnel side; the
source-subnet match in ufw is what keeps a packet arriving on any other
interface from matching these rules even if it spoofs one of those source
addresses. `system/hotspot/mango-hotspot` scopes its *own* `p2p0` rules to
DHCP/DNS only — reaching wayvnc or kdeconnect over the hotspot depends
entirely on this script's rules, not on anything mango-hotspot opens.

There is no password and no encryption anywhere in this path. Do not run
`wayvnc` with a listen address broader than `0.0.0.0`/`::` (it already is
the broadest) on the strength of the ufw rules alone — if ufw is disarmed
or its rules are missing, the server is reachable, unauthenticated, from
every interface.

## KDE Connect pairing over the tunnel

Discovery is UDP broadcast on 1716, which does not cross a WireGuard `/32`
point-to-point link. A peer reached only through the tunnel must be added
by IP: `kdeconnect-cli --refresh` won't find it, so add the address under
`customDevices` in `~/.config/kdeconnect/kdeconnectrc`'s `[General]`
section, or use `kdeconnect-cli --pair --device <id>` once the daemon has
the IP. On the hotspot's real L2 segment, broadcast discovery works
normally and needs nothing extra.
