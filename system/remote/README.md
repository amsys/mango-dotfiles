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

## Scope: wg_hetzner + hotspot only, no VNC password

Both daemons bind `0.0.0.0` — kdeconnectd has no bind-address option at
all, and wayvnc's one bind address can't cover both `wg_hetzner`
(`10.0.254.2/32`) and the hotspot's `p2p0` (transient address) at once. So
**ufw is the only boundary**, not the bind address. `install.sh` in this
directory adds the `wg_hetzner` rules; the hotspot needs nothing extra —
`system/hotspot/mango-hotspot` already opens a blanket `allow in on p2p0`
while it's up.

No VNC password is configured. Anyone who reaches port 5900 over
`wg_hetzner` or the hotspot gets full keyboard and mouse — WireGuard's
cryptokey routing (only an authenticated peer's traffic ever emerges from
`wg_hetzner`) is the only auth layer, by deliberate choice, the same trust
model `~/brain/firewall.md` already uses for ssh and paseo on this
interface.

## KDE Connect pairing over the tunnel

Discovery is UDP broadcast on 1716, which does not cross a WireGuard `/32`
point-to-point link. A peer reached only through `wg_hetzner` must be added
by IP: `kdeconnect-cli --refresh` won't find it, so add the address under
`customDevices` in `~/.config/kdeconnect/kdeconnectrc`'s `[General]`
section, or use `kdeconnect-cli --pair --device <id>` once the daemon has
the IP. On the hotspot's real L2 segment, broadcast discovery works
normally and needs nothing extra.
