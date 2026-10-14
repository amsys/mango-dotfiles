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

`wlopm --off '*'` stays the mechanism. On its own the gap was real: the
panels go dark, but the session stays unlocked, so a person at the laptop
could still type into it. Panel blanking is privacy from onlookers, not
authentication — `mango-physlock` (below) closes the authentication half.

`hypr/hypridle.conf`'s 10-minute blank listener calls
`remote.sh --idle-wake` on resume rather than a bare `wlopm --on '*'`: VNC
input resets hypridle's idle timer the same as local input does, so a bare
`--on` would relight the panels on the next remote keystroke after any
10-minute quiet spell. `--idle-wake` is a no-op while
`wayvnc-privacy.service` already has the panels down for a real client.

### Physical lock during a remote session

`mango-physlock.service` (`system/remote/physlock.py`, root-owned) closes
the gap above: for as long as a VNC client is connected, it holds an
`EVIOCGRAB` on every physical keyboard, touchpad, mouse and touchscreen —
never the power button, the lid switch, or any device under
`/sys/devices/virtual/input` (uinput; kdeconnectd's own virtual devices
live there, and wayvnc drives the session through the Wayland
virtual-keyboard protocol, not evdev at all, so remote input is never at
risk of being grabbed by mistake). mango gets no events from the grabbed
devices; the remote user is unaffected.

The exclusion is `/sys/devices/virtual/input` only, not all of
`/sys/devices/virtual`. BlueZ puts Bluetooth LE keyboards and mice under
`/sys/devices/virtual/misc/uhid` (on this laptop: the ThinkPad Bluetooth
keyboard and the RAPOO mouse). They are physical devices, so the helper
grabs them. A program that makes its own uhid device would also be
grabbed; nothing on this machine does that for remote input.

Root is required because the session user is deliberately not a member of
`input` (the same policy `system/vault/` applies to the power button
alone) — this is the one piece of this feature that cannot run as the
session user. `remote.sh --privacy-watch` starts and stops the helper over
the two fixed, no-argument sudoers verbs in `mango-physlock.sudoers`
(`systemctl start|stop mango-physlock.service`), never anything broader.

Two ways the grab comes off, both ending locked, never unlocked:

- **VNC disconnect.** `remote.sh --privacy-watch`'s disconnect handler
  locks the session first (`sleep-lock.py lock --until-locked`, confirming
  the lock rather than blocking until it is unlocked again — see that
  script's own module doc) and only stops the helper, and only turns the
  panels back on, once the lock is confirmed. A lock that cannot be
  confirmed leaves the helper running and the panels off — SPEC.md §7.2
  step 4's "no unlocked gap", read literally: a lock that fails must never
  be allowed to look like success to anything downstream of it.
- **Esc held 2s at the laptop (`SPEC.md` §7.3).** The person at the
  laptop can always take control back, with no frame of session content
  ever shown. Because the helper holds the grab, only it sees this key —
  mango and the remote user do not; a short press does nothing, so a
  bumped key cannot end the remote session. On the hold, the helper runs
  `remote.sh --takeover` as the session user (`runuser`, with the
  session's own `XDG_RUNTIME_DIR`/`WAYLAND_DISPLAY`, discovered from
  `/run/user/<uid>` since a root service inherits neither): that locks
  the session (again `lock --until-locked`), stops wayvnc and its
  privacy watcher, restores any pulled tag, destroys the headless
  output, and turns the panels back on — in that order, so the remote
  client loses its view only once the lock screen is already the only
  thing any output can show. The helper releases its grab only once
  `--takeover` exits 0; a failed or slow attempt (15s budget) keeps the
  grab, and a second Esc hold can retry.

Residual gaps, not closed by any of this:

- **The crash window between grab release and the fallback lock.**
  `mango-physlock.service`'s `ExecStopPost` always runs `physlock.py
  --on-stop`, which locks the session the same way (`lock
  --until-locked`) — on a clean stop or a crash, so the unit going away
  for any reason still ends locked (SPEC.md §7.2 step 5's fail-closed
  rule). But `ExecStopPost` only starts once `ExecStart`'s own process has
  actually exited, and that exit is exactly when the grab comes off. The
  window between "grab released" and "`--on-stop`'s lock has actually
  landed" is real, bounded by how fast `hyprlock`/`swaylock` can spawn and
  confirm — not provably zero.
- **A killed lock client's effect on `ext-session-lock-v1` is
  unverified on this `mangowc` build.** So no unit stop may kill a locker.
  The callers of `lock --until-locked` are units (`wayvnc-privacy` runs
  `--privacy-watch`, `mango-physlock` runs `--takeover`), and a unit stop
  kills its whole cgroup; `start_new_session` does not leave the cgroup.
  A first build broke this: the takeover stopped `wayvnc-privacy` and so
  killed the lock that the disconnect path had started in that unit.
  `lock --until-locked` now runs itself in its own transient user scope
  (`systemd-run --user --scope`, unit `mango-lock-<pid>`), and the
  locker, its stdout drain (`_detach_stdout_drain`) and its unpause
  watcher live there. `sleep-lock.py test` checks the locker's cgroup.
  Without a user bus it runs in the caller's cgroup and logs that.
  The scope move works only from inside the user manager's own tree: from
  the system unit `mango-physlock` it fails (the common cgroup ancestor is
  the root cgroup, writable only by root). So `physlock.py` runs every
  user-side command through `systemd-run --user --wait --pipe`
  (`user_session_cmd`); `--wait` returns the command's exit code. Ending
  the VNC set by any path (client disconnect, toggle off, a wayvnc crash)
  locks and stops `mango-physlock` before a panel comes on; when no lock is
  confirmed, the panels stay dark and the grab stays on. The
  one place that still kills a locker on purpose is the suspend-path
  fallback (`_fallback_to_swaylock`), which kills an unconfirmed hyprlock.
- **Confirming a lock this call did not start is a settle window, not a
  log line.** `do_lock_until_locked()` no longer trusts an already-running
  hyprlock/swaylock on sight (`running_locker_pid() is not None` proves a
  process exists, not that it ever reached `ext_session_lock_v1.locked`)
  — it now requires that pid to stay the active locker for
  `LOCKER_SETTLE_SEC` before trusting it, and starts its own (genuinely
  confirmed) locker if that pid loses the race instead. This rules out
  the fast-failing collision case, not a locker that is merely slow to
  confirm past the settle window while never losing the pid race — this
  row did not audit how `ext_session_lock_v1.locked` timing compares to
  that window on real hardware.
- **SysRq is untested.** Whether an `EVIOCGRAB`'d keyboard still reaches
  the kernel's SysRq handler was an open item in the original spec and
  stays open here — nothing in this row checked it.
- **A USB keyboard plugged in mid-session** is picked up by
  `physlock.py`'s udev watch (or, failing that, its 2s poll) — see the
  module's own doc for the netlink details and its poll-only fallback.
  The kernel uevent socket is in the main loop's own `select()` set, so an
  input "add" event starts a rescan at once.
  A keyboard that *disconnects and reconnects* while the daemon runs is a
  separate case `prune_disconnected()` exists for: Linux tends to reissue
  a freed `/dev/input/eventN` number to the very next device that
  registers, so without pruning a stale `grabbed` entry for the vanished
  device, a replug landing on that same path would never be re-probed.

**Flash test (required before this ships, SPEC.md §7.3):** record the
panel with a phone in slow motion during a takeover's last three steps
(lock confirmed -> stop wayvnc -> panels on), ten times, with a bright
window that was on screen before the remote session started. One frame of
old content is a failure. `grim` reads the compositor, not the panel
scanout, so only a camera can prove what the panel actually showed. Not
run as part of this row (see its own report) — it needs the real
hardware, a real VNC client, and a person holding Esc, none of which a
`--selftest` can stand in for.

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

## Remote commands

The phone's "Run command" plugin holds two entries. They are **not** in this
repo: kdeconnectd writes them to
`~/.config/kdeconnect/<device-id>/kdeconnect_runcommand/config`, a path keyed
by the paired device's id. Same treatment as `keepassxc.ini` — the content is
recorded here, not linked:

- **Toggle Remote Access** — `/bin/bash ~/.config/ironbar/scripts/remote.sh
  --toggle-vnc`. The same thing the bar pill's left click does, from the
  phone, so VNC can be armed before you are at the machine.
- **Sleep** — `systemctl --user stop mango-keepawake.service; mango-bard
  refresh keepawake; systemctl suspend`. Stops the keep-awake inhibitor
  first. Suspend does not need that (`mango-keepawake` blocks
  `idle:handle-lid-switch`, never `sleep`), but a keep-awake left on would
  keep blocking idle suspend after the wake, so the command clears it. The
  `mango-bard refresh` call is what stops the bar's coffee-cup pill from
  showing a stale ON state — that pill has no poll, it refreshes only on the
  edge (`ironbar/scripts/keepawake.sh`).

The plugin runs each command through `/bin/sh -c`, so `~`, `;` and `$VAR`
all expand. The daemon owns the file, so edit it while
`kdeconnectd.service` is stopped — whether a running daemon overwrites an
outside edit is untested here.

## KDE Connect pairing over the tunnel

Discovery is UDP broadcast on 1716, which does not cross a WireGuard `/32`
point-to-point link. A peer reached only through the tunnel must be added
by IP: `kdeconnect-cli --refresh` won't find it, so add the address under
`customDevices` in `~/.config/kdeconnect/kdeconnectrc`'s `[General]`
section, or use `kdeconnect-cli --pair --device <id>` once the daemon has
the IP. On the hotspot's real L2 segment, broadcast discovery works
normally and needs nothing extra.
