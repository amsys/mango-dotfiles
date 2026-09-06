# The bar — ironbar + mango-bard

The bar is [ironbar](https://github.com/JakeStanger/ironbar). A Rust
daemon, `mango-bard` (`ironbar/bard/`), collects all system data. It sends
the data to the bar through the IPC variables of ironbar. The daemon runs
as the systemd user unit `mango-bard.service`.

## Launch

`mango/config.conf` starts `ironbar/scripts/start.sh` at login. The script:

1. Runs `mango-bard gen-config` to write `~/.config/ironbar/config.json`.
   The command generates the config from the monitor list. It makes one
   bar for each output. It also makes a fallback bar for an output that
   you plug in later. The config is a real file, never a symlink.
2. Exports `LD_PRELOAD=~/.local/lib/mango/fast-tooltips.so` if the shim is
   built. The shim decreases the fixed 500 ms tooltip delay of GTK3
   (`MANGO_TOOLTIP_DELAY_MS`, default 80).
3. Starts `ironbar`.

matugen writes `~/.config/ironbar/style.css`. Edit
`matugen/templates/ironbar/style.css` instead. See
[theming.md](theming.md).

## Modules

The layout is grouped by domain. See
[statusbar-layout.md](statusbar-layout.md) for the full normative spec. It
gives the module order, the width invariants and the reason for the
position of each group.

Start (launcher → tags → focus):

| Pill | Shows | Click |
|---|---|---|
| spark | launcher star | app launcher |
| *(9 tag pills + overview)* | the tags of each monitor. Left-click views a tag. Scroll moves through the tags. Hover lists the windows of a tag | — |
| win | the title of the focused window. A kitty window title has a repo prefix (kitty/repo-title.py) | scroll: brightness |

A `HEADLESS-*` bar is different. This bar belongs to the wayvnc capture
output. It has no clock, no tray and no audio pills. Its `start` group is
a remote-control strip, not the tags of that output.

The strip begins with one private pill. A click on that pill sends any
pulled tag back. After it, the strip shows the name and the nine tag pills
of each physical monitor. A click pulls a tag. It does not view a tag. See
[remote.sh](../ironbar/scripts/remote.sh) and the `HEADLESS` branch of
`genconfig.rs::build()`.

Center (time):

| Pill | Shows | Click |
|---|---|---|
| clock | the time. The popup shows the local clock and the world clocks | — |
| date | the date. The popup shows a month calendar | focus the calendar app |
| pomo | pomodoro state | left: start/pause; right: mute |

End (tray → resources → tools → audio → connectivity → session):

| Pill | Shows | Click |
|---|---|---|
| tray | hidden while the drawer is closed | the app window or the menu. It goes to an open window first |
| traytoggle | drawer trigger | open/close the tray drawer |
| keepass | KeePassXC lock state | show/hide KeePassXC |
| sysload | two stacked rows. The top row has the cpu icon and gauge. The bottom row has the memory icon and gauge. One popup covers both rows. It gives the exact %, the per-core bars, the temperature and the top processes. It also gives the RAM and swap meters, the page faults and the DIMM data | `btop` |
| battery | the icon, the gauge and a terminal-nub cap. The popup gives the health, the cycle count, the watts, the time estimate and the power attribution | left: power mode toggle; right: powertop |
| devload | two stacked rows. The top row shows the Claude usage. The bottom row shows a docker container count and a pending-update count. A count of zero is hidden. One popup covers all three | claude row right: usage settings; docker cell right: docker menu; updates cell left: run `arch-update` |
| colorpicker | pixel color picker | pick a color (`hyprpicker`) |
| darkmode | light/dark scheme toggle | toggle scheme |
| snip | screenshot | left: region; right: window |
| inhibit | keep-awake state | toggle keep-awake |
| music | the current track on two lines. The dim artist is above the title. The pill is hidden if nothing is loaded | play/pause |
| volume, mic | audio | `pavucontrol-qt`; scroll: volume |
| net-spinner, wifi, eth, netsec | network state and a security grade | menus; see below |
| hotspot | hidden while inactive | left: menu; right: toggle |
| remote | the popup gives a read-only status: VNC on or off, the blank state of the local screens, the pulled tag, and KDE Connect on or off | left: VNC toggle (this also starts KDE Connect); right: KDE Connect toggle. To cycle the pull, use the SUPER+CTRL keybinds, or click a tag pill on the remote-control strip of the headless bar |
| bluetooth | devices | right: `blueman-manager` |
| power | — | session menu |

## Hover popups

The popups are ironbar `popup` widgets. They are bound to variables that
the daemon owns. A hover opens a popup. A click does not open it. Each
pill reports `hover enter|exit` to the daemon. The daemon debounces the
report: an 80 ms open delay, and a 200 ms grace to cross into the popup.

The daemon then refreshes that collector and toggles the popup. It writes
only the variables that changed. It debounces these writes at 150 ms.

## The daemon CLI

```
mango-bard run                 # the daemon (systemd runs this)
mango-bard ping | stats        # liveness and internals
mango-bard refresh <topic>     # force one collector to refresh
mango-bard gen-config [--out PATH]
mango-bard pomo <verb>         # click, mute, and the other pomodoro verbs
mango-bard hover ...           # used by the generated config
```

## Module scripts

The daemon does the data collection. The interactive actions stay in
`ironbar/scripts/`:

| Script | Function |
|---|---|
| `wifi-menu.sh` | rofi Wi-Fi network menu |
| `eth-toggle.sh` | ethernet up/down |
| `net.sh` | network security grade details and click actions |
| `docker-menu.sh` | rofi docker actions |
| `hotspot.sh` | hotspot menu and toggle |
| `keepawake.sh` | toggles `mango-keepawake.service` (idle + lid inhibit) |
| `remote.sh` | `--toggle-vnc` switches `wayvnc.service`, the keep-awake and the panel blanking on a virtual (headless) output. It also starts `kdeconnectd.service`. `--toggle-kdeconnect` switches `kdeconnectd.service` alone. `--pull`/`--pull-next`/`--pull-prev`/`--restore` move the tag of a physical monitor onto the virtual output and back |
| `darkmode.sh` | light/dark toggle, then re-theme |
| `clock.sh --calendar` | focus or start the calendar app |
| `tray-click.sh` | tray `on_click_left`. It jumps to an already-open window. If there is none, it sends SNI `Activate` |
| `tray-drawer.sh` | toggles the tray drawer open/closed |
| `tooltip.sh` | shared popup text helpers. The scripts above source this file |

## Pomodoro

The pomodoro engine is in the daemon (`pomo.rs`). Left-click the pomo pill
to start or pause the timer. Right-click it to mute for 30 minutes
(`MANGO_POMODORO_MUTE`). Press `SUPER+SHIFT+T` to name the current task.
The durations come from `MANGO_POMODORO` (`work,short,long,cycles`,
default `25,5,15,4`).

A phase change rings once. It also raises a critical notification that
replaces the previous one. A break opens a full-screen overlay
(`mango/scripts/focus-break.sh`). `focus-note.sh` keeps a parking lot.
`focus-review.sh` runs the weekly review.

The pomodoro pauses at every screen lock, not only at suspend. `SUPER+L`
and the idle-timeout lock of hypridle both use `sleep-lock.py lock` (see
`docs/power.md`). For this reason a phase boundary that occurs while the
screen is locked never rings the bell.

`focus-break.sh` also refuses to open its overlay while `swaylock` runs.
The overlay closes after 120s (`OVERLAY_TIMEOUT`) if it cannot take the
keyboard input. `SUPER+SHIFT+Escape` kills a stuck rofi by hand.

## Self-checks

`mango-bard` has unit tests. Run `cargo test` in `ironbar/bard/`. The
scripts keep the convention of the repo: `net.sh --selftest`, and a `test`
subcommand on `clock.sh`, `docker-menu.sh`, `hotspot.sh`,
`tray-click.sh` and `tray-drawer.sh`.
