# The bar — ironbar + mango-bard

The bar is [ironbar](https://github.com/JakeStanger/ironbar). A Rust daemon,
`mango-bard` (`ironbar/bard/`), collects all system data and feeds it to the
bar through ironbar's IPC variables. The daemon runs as the systemd user
unit `mango-bard.service`.

## Launch

`mango/config.conf` starts `ironbar/scripts/start.sh` at login. The script:

1. Runs `mango-bard gen-config` to write `~/.config/ironbar/config.json`.
   The config is generated per monitor list (one bar per output, plus a
   fallback bar for outputs plugged in later), so it is a real file, never a
   symlink.
2. Exports `LD_PRELOAD=~/.local/lib/mango/fast-tooltips.so` when the shim is
   built. The shim shortens GTK3's fixed 500 ms tooltip delay
   (`MANGO_TOOLTIP_DELAY_MS`, default 80).
3. Starts `ironbar`.

`~/.config/ironbar/style.css` is matugen output — edit
`matugen/templates/ironbar/style.css` instead (see
[theming.md](theming.md)).

## Modules

Layout is grouped by domain — see
[statusbar-layout.md](statusbar-layout.md) for the full normative spec
(module ordering, width invariants, and why each group sits where it does).

Start (launcher → tags → focus):

| Pill | Shows | Click |
|---|---|---|
| spark | launcher star | app launcher |
| *(9 tag pills + overview)* | per-monitor tags — left-click views a tag, scroll moves through tags, hover lists a tag's windows | — |
| win | focused window title | scroll: brightness |

Center (time):

| Pill | Shows | Click |
|---|---|---|
| clock | time; popup: local + world clocks | — |
| date | date; popup: month calendar | focus the calendar app |
| pomo | pomodoro state | left: start/pause; right: mute |

End (tray → resources → tools → audio → connectivity → session):

| Pill | Shows | Click |
|---|---|---|
| tray | — | app menus |
| cpu | usage; popup: per-core bars, temperature, top processes, recent peaks from atop | `btop` |
| memory | usage; popup: RAM/swap meters, page faults, top processes, DIMM data | `btop` |
| docker | container count; popup: container list | right: docker menu |
| battery | charge; popup: health, cycle count, watts, time estimate, power attribution | left: power mode toggle; right: powertop |
| claudebar | Claude usage | right: usage settings |
| tools | hover-expandable drawer: colorpicker, darkmode, snip | hover to reveal |
| inhibit | keep-awake state | toggle keep-awake |
| music | MPRIS track | play/pause |
| volume, mic | audio | `pavucontrol-qt`; scroll: volume |
| net-spinner, wifi, eth, netsec | network state and a security grade | menus; see below |
| hotspot | hidden unless active | left: menu; right: toggle |
| remote | wayvnc + KDE Connect state | toggle |
| bluetooth | devices | right: `blueman-manager` |
| power | — | session menu |

## Hover popups

Popups are ironbar `popup` widgets bound to daemon-owned variables. Hover
opens them, not click: each pill reports `hover enter|exit` to the daemon,
which debounces (80 ms open delay, 200 ms grace to cross into the popup),
refreshes that collector, and toggles the popup. The daemon writes only
changed variables, debounced at 150 ms.

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

The daemon covers the data collection. Interactive actions stay in
`ironbar/scripts/`:

| Script | Function |
|---|---|
| `wifi-menu.sh` | rofi Wi-Fi network menu |
| `eth-toggle.sh` | ethernet up/down |
| `net.sh` | network security grade details and click actions |
| `docker-menu.sh` | rofi docker actions |
| `hotspot.sh` | hotspot menu and toggle |
| `keepawake.sh` | toggles `mango-keepawake.service` (idle + lid inhibit) |
| `remote.sh` | toggles `wayvnc.service` + `kdeconnectd.service` |
| `darkmode.sh` | light/dark toggle, then re-theme |
| `clock.sh --calendar` | focus or start the calendar app |
| `tooltip.sh` | shared popup text helpers, sourced by the scripts above |

## Pomodoro

The pomodoro engine lives in the daemon (`pomo.rs`). Left-click the pomo
pill to start or pause, right-click to mute for 30 minutes
(`MANGO_POMODORO_MUTE`), and use `SUPER+SHIFT+T` to name the current task.
Durations come from `MANGO_POMODORO` (`work,short,long,cycles`, default
`25,5,15,4`). Phase changes ring once and raise a critical notification
that replaces its predecessor. Breaks open a full-screen overlay
(`mango/scripts/focus-break.sh`); `focus-note.sh` keeps a parking lot and
`focus-review.sh` runs the weekly review.

## Self-checks

`mango-bard` has unit tests (`cargo test` in `ironbar/bard/`). The scripts
keep the repo convention: `net.sh --selftest`, and `test` subcommands on
`clock.sh`, `docker-menu.sh`, and `hotspot.sh`.
