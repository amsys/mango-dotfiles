# Remote access: wayvnc + KDE Connect

## What this is

One ironbar pill (`remote`, right end of the bar) drives four units. The
left click starts and stops the VNC set (wayvnc, wayvnc-privacy,
mango-keepawake) and starts kdeconnectd with it. The right click starts and
stops kdeconnectd alone. A left click that turns VNC off leaves kdeconnectd
running:

- **wayvnc** — VNC server, the screen. Talks to mango directly (wlr-screencopy
  + virtual pointer/keyboard), no portal involved.
- **kdeconnectd** — KDE Connect daemon, input/clipboard/files/notifications
  from a paired phone. Has no screen view of its own. It is useful without
  VNC, so it has its own switch: the pill's right click.
- **mango-keepawake** — blocks idle suspend and lid-switch suspend for as
  long as remote access is armed, so a 15-minute idle timeout does not end
  the session out from under a remote user. Stopped again on VNC toggle-off,
  but only if this pill is what started it — a keep-awake you had already
  switched on yourself from its own pill is left alone.
- **wayvnc-privacy** — watches for a connected VNC client and blanks every
  physical output for as long as one is connected, so nothing typed or
  shown remotely is readable at the laptop. See "Panel blanking" below.

None is enabled at login — `install-config.sh` links all four units but does
not `systemctl --user enable` them (see `LINK_ONLY_UNITS` there). The pill's
click handlers (`ironbar/scripts/remote.sh --toggle-vnc` on the left,
`--toggle-kdeconnect` on the right) are the only thing that starts or stops
them. Eco power mode also turns VNC off when no client is connected; it does
not touch kdeconnectd, and leaving eco does not start VNC again.

## The virtual output

wayvnc doesn't capture a physical panel. `--toggle-vnc` creates a headless
output (`mmsg dispatch create_virtual_output`, named `HEADLESS-<n>` —
wlroots increments `<n>` on every create and never reuses a name, so
`remote.sh` re-discovers it after each create rather than hardcoding it) and
pins wayvnc to it (`--vnc-exec`). The remote user works entirely on that
output; the physical panels stay blanked by `wayvnc-privacy` (below) rather
than shown. `--toggle-vnc` also regenerates and reloads the bar config, so
the virtual output gets an ironbar bar of its own — a remote-control strip,
not a copy of a normal bar (see "Moving a tag onto it" below) — for as long
as VNC is armed, and loses it again on toggle-off.

Toggle-off moves every window still on the virtual output to the first
physical monitor before it destroys that output. mango does not move them
itself, so a window left there would be lost. Toggle-off then turns the
physical panels back on.

wayvnc resizes a headless output to match the connecting client automatically
(built in since 0.10.1, on by default — `-R/--disable-resizing` opts out,
and `--vnc-exec` doesn't pass it; it only ever resizes a *headless* output,
never a physical one). No sizing code needed here.

Nothing is on the virtual output at first — it's empty desktop. Move a tag
onto it with:

- **The headless bar itself** — its `start` row is a remote-control strip
  (`genconfig.rs::build()`'s `HEADLESS` branch, `remote_pills()`): a private
  pill first (click: send any pulled tag back — `--restore`), then each
  physical monitor's name and its nine tag pills. Clicking a pill pulls
  that tag (`remote.sh --pull <mon> <n>`) instead of viewing it. This is
  the mouse path — the earlier design put a clickable grid in the `remote`
  pill's own popup on the *physical* bars, which a remote viewer can never
  reach while those panels are blanked; it was deleted for exactly that
  reason (see IRONBAR.md's T-remote-popup entry) and rebuilt here, on the
  one bar that actually is reachable.
- **`SUPER+CTRL+Next`/`SUPER+CTRL+Prior`** (`--pull-next`/`--pull-prev`) —
  cycle through every occupied tag on both physical monitors without a
  mouse. Reachable from a client that can only send modifiers plus
  Tab/Esc/PgUp/PgDown/Home.
- **`SUPER+CTRL+Home`** (`--restore`) — send the pulled tag back to its own
  monitor.

Only one tag is pulled at a time — pulling a second sends the first back
first. The candidate list for the cycle comes from `mmsg get all-clients`,
never `all-tags`: that command's own `client_count` field reports the
compositor-wide client total for every occupied tag, not the tag's own
count (verified live — worth reporting upstream, not a bug in this repo).
Restoring is safe to call twice, and also runs automatically on VNC client
disconnect (see "Panel blanking") and on VNC toggle-off.

## Panel blanking

`wayvnc-privacy.service` watches `wayvncctl event-receive` for
`client-connected`/`client-disconnected` and runs `wlopm --off '*'` /
`wlopm --on '*'` on the edges, restoring only the outputs that were on
before.

This is safe *because* of how mango implements DPMS: a `wlopm --off`
output keeps its real geometry and keeps feeding wlr-screencopy — confirmed
live, wayvnc kept streaming a DPMS-off output to a connected client
throughout this feature's own development. `mango/scripts/rescue-outputs.sh`
documents the same fact from the other side (an `only_sleep` monitor is not
the "frozen desktop" state it watches for). A crashed or killed watcher
cannot leave the panels dark forever — the unit's `ExecStopPost` always runs
`remote.sh --privacy-restore`, which is safe to run twice.

### Why a lock, not just a blank

A swaylock over the physical panels only is not possible here, for two
reasons. First, `ext-session-lock-v1` (the protocol mango exposes, and the
one swaylock uses) needs a lock surface on every output, `HEADLESS-<n>`
included, or the compositor kills the client — there is no per-output
opt-out, in the protocol or in swaylock's own flags. A lock would shut the
remote user out of the session it exists to serve. Second, mango has one
seat: wayvnc drives the session through a virtual keyboard and pointer on
that same seat, so no lock surface, even one drawn on `eDP-1` alone, can
apply to the panels without also catching the remote input.

`wlopm --off '*'` stays the mechanism. The gap this leaves is real: the
panels go dark, but the session stays unlocked, so a person at the laptop
can still type into it. Panel blanking is privacy from onlookers, not
authentication.

`hypr/hypridle.conf`'s 10-minute blank listener calls
`remote.sh --idle-wake` on resume rather than a bare `wlopm --on '*'`: VNC
input resets hypridle's idle timer the same as local input does, so a bare
`--on` would relight the panels on the next remote keystroke after any
10-minute quiet spell. `--idle-wake` is a no-op while
`wayvnc-privacy.service` already has the panels down for a real client.

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

"Off means nothing listening" needs two overrides on top of the toggle
unit: `kdeconnect/org.kde.kdeconnect.daemon.desktop` disables the XDG
autostart, and `kdeconnect/org.kde.kdeconnect.service` disables D-Bus
activation of the daemon (Chromium calls the `org.kde.kdeconnect` bus
name at each Chromium start; without the override that call started
kdeconnectd outside the unit). With both in place, the bar toggle's
`systemd/kdeconnectd.service` is the only path that starts kdeconnectd
automatically. `/usr/share/applications/org.kde.kdeconnect.daemon.desktop`
still runs the binary directly, but it is `NoDisplay=true`, so only a
deliberate manual launch reaches it.

## KDE Connect pairing over the tunnel

Discovery is UDP broadcast on 1716, which does not cross a WireGuard `/32`
point-to-point link. A peer reached only through the tunnel must be added
by IP: `kdeconnect-cli --refresh` won't find it, so add the address under
`customDevices` in `~/.config/kdeconnect/kdeconnectrc`'s `[General]`
section, or use `kdeconnect-cli --pair --device <id>` once the daemon has
the IP. On the hotspot's real L2 segment, broadcast discovery works
normally and needs nothing extra.
