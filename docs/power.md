# Power

## Power modes (`powermode.sh`)

There are three modes. A left click on the battery pill changes between
**full** and **eco**. The system enters and leaves **battery** mode
automatically with the AC cable.

- **full** — The CPU EPP, the turbo, and the platform profile are at the
  maximum. The PCI and NVMe runtime power management is off.
- **battery** — The system uses this mode when you disconnect the cable.
  The CPU stays responsive with `balance_power` and turbo on. The mode also
  saves power on the I/O side: PCI and NVMe runtime PM, laptop-mode
  writeback, Wi-Fi power save, and a dimmed backlight. The mode pauses
  nothing and stops nothing. Below `PM_BAT_ECO_PCT` (40%), the mode changes
  itself to eco.
- **eco** — All the knobs are at the minimum, and the backlight is dimmer.
  A Docker container gets one of three results. A container with a live
  `docker exec` or build keeps running. The mode pauses a container that
  hosts only a `PM_ECO_BUSY_PROCS` process, and it stops an idle container.
  Before the heavy steps, eco waits for the open coding-agent sessions to go
  quiet (measured by CPU ticks), unloads the ollama models, and pauses
  hermes. The battery drain has priority over the wait: below
  `PM_ECO_DRAIN_FORCE_PCT`, eco continues immediately.

A manual click sets an override. The override stays until the cable state
changes. `battery-guard.sh` also looks for a **weak charger**. A weak
charger is an AC source that is online while the battery still drains. It is
also a USB-C source below `PM_WEAK_MIN_W`. The guard then forces eco and
shows a critical notification until the charger recovers.

`mango/powermode.conf` holds every knob. The repo tracks this file and
symlinks it. A comment is above each key. Your edits apply at the next mode
switch.

```
powermode.sh status | full | battery | eco   # status / force a mode
powermode.sh test                            # decision-table self-check
```

Only `/usr/local/bin/mango-powermode` writes the root-owned sysfs knobs
(EPP, turbo, platform profile, PCI PM, and more). Install this helper one
time:

```bash
sudo system/powermode/install.sh
```

The user can write to `powermode.conf`, thus the root helper never reads it.
`powermode.sh` changes the config into `KEY=value` lines. It sends the lines
to the helper on stdin. The helper compares each value with a closed set
before it writes to sysfs. Mode switches also work before this install. They
only skip the root-owned knobs.

## Low battery (`battery-guard.sh`)

`battery-guard.sh` is an `exec-once` watcher. It polls every 30 s. The
thresholds come from `MANGO_BATTERY` (default `20,10,5,3`):

| Level | Action |
|---|---|
| 20% | a notification and a soft chime |
| 10% | a persistent critical notification and an alarm, repeated every 5 min |
| 5% | a 60 s countdown, then `systemctl suspend` |
| 3% | suspends even when a task is in progress |
| 2% | the `PercentageAction` of UPower. This is the last resort. It is normally unreachable |

The cable connection cancels a running countdown and re-arms each tier. A
tier re-arms two percent above the level where it fired. A battery on a
threshold thus does not chatter.

At 5%, the guard reads `mango/scripts/busy.sh`. It waits while a pacman
transaction or a fresh partial download is in progress. At 3%, it suspends
in all conditions. A critical notification stays until you dismiss it. The
`[urgency=critical]` rule of mako sets `default-timeout=0`.

## The power button

| Gesture | Action |
|---|---|
| tap (< 1 s) | opens the session menu (`powermenu.sh`) |
| hold 1–3.5 s, release | suspends |
| hold ~4 s | the firmware cuts the power (fixed in hardware) |

Shutdown is a menu button. It is not a hold tier. A hold near 4 s races the
firmware cut.

`mango/scripts/powerkey.py` reads the power-button input device. It measures
the time from the press to the release. It is a watcher, because mango has
no release binds. Also, the long-press threshold of logind (5 s) is after
the firmware cut. logind must not handle the key. See the checklist in
[install.md](install.md) (`HandlePowerKey=ignore`).

## The session menu (`powermenu.sh`)

`powermenu.sh` is a wrapper around `wlogout`. It shows a warning first when
`busy.sh` reports a package transaction or an unfinished download. The
warning informs you, but it does not block you. The script centers a
fixed-size panel on the active monitor. matugen writes the stylesheet
(`matugen/templates/wlogout/style.css`). The button icons are SVG files in
`wlogout/icons/`, because wlogout shows label text at one fixed size.

## Charger chime (`ac-watch.sh`)

`ac-watch.sh` is a second `exec-once` watcher. It waits for udev
power-supply events. It shows a light notification and plays a sound at each
plug or unplug event. It does not play the sound while the default sink is
muted. It also sends the new cable state to `powermode.sh`.

## Lock before sleep (`sleep-lock.py`)

The systemd user unit `mango-sleep-lock.service` locks the screen before a
suspend. It also pauses the pomodoro during the sleep. It holds a logind
delay inhibitor. It releases the inhibitor when `swaylock --ready-fd`
confirms the lock, or after a limit of 3 s. It covers each suspend path, and
this includes the lid close.

This task is not the job of hypridle. A compositor can have no Hyprland
lock-notify protocol. On such a compositor, hypridle releases its sleep
inhibitor immediately after it starts the lock command. The screen is not
yet locked at that time. `hypr/hypridle.conf` gives the full explanation.

The `sleep-lock.py lock` subcommand of the same file extends the pause to
each *lock*, not only to a suspend. `SUPER+L` and the 300 s idle-timeout
listener of hypridle call it instead of a bare `swaylock`. Before this
change, the pomodoro continued while the screen was locked. A phase boundary
in that time started a full-screen rofi overlay that could not take keyboard
input after the unlock. The Pomodoro section of `docs/bar.md` has the
symptom and the rest of the fix (the swaylock guard and the timeout in
`focus-break.sh`). `lock` looks for a swaylock process first and does
nothing if it finds one, thus you can bind it in all conditions.

`sleep-lock.py test` runs a self-check. The check includes the pause,
swaylock, and unpause call order of `lock`. `SIGUSR1` on the running daemon
does a lock-and-pause rehearsal without a suspend.

## Keep-awake

The inhibit pill of the bar toggles `mango-keepawake.service`. This unit is
a `systemd-inhibit` block on the idle handling and the lid-switch handling.
logind obeys the lid part only with `LidSwitchIgnoreInhibited=no`. See the
checklist.

## Display rescue (`rescue-outputs.sh`)

The symptom is this: the desktop stops to draw and stays frozen on the last
frame. The input still works. The keybinds fire, and a password that you
type reaches the lock screen. Only the painting stops. This is the
`selmon == NULL` state of mango: each output is disabled, thus the
compositor has no surface to draw on. This is not a hang, and it is not a
crash.

A source-level check on 2026-08-26 used the exact installed `mangowm-git`
build. The check shows that `wlopm --off '*'` is **not** the cause (DPMS,
the 600 s screen-off in `hypridle.conf`). A DPMS-off monitor keeps its real
geometry, stays only disabled, and stays in the layout. The remaining
suspects apply a `zwlr_output_management_v1` configuration with an output
disabled. `wdisplays` is installed, and it does this. A `disable_monitor` or
`toggle_monitor` dispatch is also a suspect, but no keybind here uses one.

The recovery needs no compositor restart, and it loses no applications.
mango keeps a disabled output on its monitor list. To enable the output
again is a plain IPC call. There are three layers:

1. **`mango-outputs.service`** polls for the frozen state. It enables the
   output again automatically, usually in approximately 15 s.
2. **`SUPER+SHIFT+o`** runs the same rescue immediately. This is the manual
   escape. It also works when the watchdog is dead, because the input still
   responds during the freeze.
3. **A TTY** (`Ctrl+Alt+F3`) is the last resort. Run
   `~/.config/mango/scripts/rescue-outputs.sh`. Do this before you use
   `systemctl restart sddm`. That restart loses each open application.

```
rescue-outputs.sh        # one-shot: check, rescue if frozen
rescue-outputs.sh watch  # poll loop (what the service runs)
rescue-outputs.sh test   # self-check, no compositor needed
```

Two `windowrule=` rules used fractional sizes. These rules crashed mango in
this state. `mango.c:1827-1834` reads the monitor with no NULL guard to
resolve a fraction. `config.conf` now uses literal pixel sizes for these two
rules. A crash there kills each running application and the compositor.

## Power attribution (`system/rapl/`)

`sudo system/rapl/install.sh` shows the per-domain power in the battery
popup:

- A udev rule gives the `wheel` group read access to the RAPL energy
  counters. By default, only root can read RAPL. The reason is the PLATYPUS
  side channel (CVE-2020-8694). This risk does not apply on a single-user
  laptop.
- A sudoers rule permits `powertop` with no arguments. A right click on the
  battery pill starts it.

powertop ranks the devices only after you run `sudo powertop --calibrate`
one time on battery. Without the install, the popup shows only the total
draw number.
