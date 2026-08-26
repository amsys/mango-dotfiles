# Power

## Power modes (`powermode.sh`)

Three modes. A left click on the battery pill cycles **full ↔ eco**.
**battery** mode is entered and left automatically with the AC cable.

- **full** — CPU EPP, turbo, and platform profile at maximum. PCI and NVMe
  runtime power management off.
- **battery** — the cable comes out. The CPU stays responsive
  (`balance_power`, turbo on), and the I/O-side savings come for free: PCI
  and NVMe runtime PM, laptop-mode writeback, Wi-Fi power save, a dimmed
  backlight. Nothing is paused or stopped. Under `PM_BAT_ECO_PCT` (40%)
  the mode escalates itself to eco.
- **eco** — everything at minimum, backlight dimmed further. Docker
  containers get one of three outcomes: a container with a live
  `docker exec` or build keeps running, one that only hosts a
  `PM_ECO_BUSY_PROCS` process is paused, and an idle one is stopped.
  Before the heavy steps run, eco waits for open coding-agent sessions to
  go quiet (measured by CPU ticks), unloads ollama models, and pauses
  hermes. Battery drain outranks the wait: below `PM_ECO_DRAIN_FORCE_PCT`
  it proceeds regardless.

A manual click sets an override that survives until the cable state
changes. `battery-guard.sh` also watches for a **weak charger** (AC online
but the battery still drains, or a USB-C source under `PM_WEAK_MIN_W`) and
forces eco with a critical notification until the charger recovers.

Every knob lives in `mango/powermode.conf`, tracked and symlinked, with a
comment over each key. Edits apply on the next mode switch.

```
powermode.sh status | full | battery | eco   # status / force a mode
powermode.sh test                            # decision-table self-check
```

The root-owned sysfs knobs (EPP, turbo, platform profile, PCI PM, ...) are
written only by `/usr/local/bin/mango-powermode`. Install it once:

```bash
sudo system/powermode/install.sh
```

`powermode.conf` is user-writable, so the root helper never reads it.
`powermode.sh` resolves the config into `KEY=value` lines and pipes them in
on stdin; the helper validates every value against a closed set before it
touches sysfs. Before this install, mode switches still work — they only
skip the root-owned knobs.

## Low battery (`battery-guard.sh`)

An `exec-once` watcher, 30 s poll. Thresholds from `MANGO_BATTERY`
(default `20,10,5,3`):

| Level | Action |
|---|---|
| 20% | notification, soft chime |
| 10% | persistent critical notification + alarm, repeated every 5 min |
| 5% | 60 s countdown, then `systemctl suspend` |
| 3% | suspends even when something is mid-flight |
| 2% | UPower's own `PercentageAction` — the last resort, normally unreachable |

Plugging in cancels a running countdown and re-arms every tier. A tier
re-arms two percent above where it fired, so a battery on a threshold does
not chatter.

At 5% the guard consults `mango/scripts/busy.sh` and defers while a pacman
transaction or a fresh partial download is in flight. At 3% it suspends
regardless. Critical notifications stick until dismissed (mako's
`[urgency=critical]` sets `default-timeout=0`).

## The power button

| Gesture | Action |
|---|---|
| tap (< 1 s) | the session menu (`powermenu.sh`) |
| hold 1–3.5 s, release | suspend |
| hold ~4 s | the firmware cuts power (fixed in hardware) |

Shutdown is a menu button, not a hold tier: a hold near 4 s would race the
firmware cut.

`mango/scripts/powerkey.py` reads the power-button input device and times
press to release. It is a watcher because mango has no release binds, and
logind's long-press threshold (5 s) sits past the firmware cut. logind must
stand down for this to work — see the checklist in
[install.md](install.md) (`HandlePowerKey=ignore`).

## The session menu (`powermenu.sh`)

Wraps `wlogout`. It warns first when `busy.sh` reports a package
transaction or an unfinished download (it informs, it does not block), and
it centers a fixed-size panel on the active monitor. The stylesheet is
matugen output (`matugen/templates/wlogout/style.css`). The button icons
are SVG files in `wlogout/icons/`, because wlogout renders label text at
one fixed size.

## Charger chime (`ac-watch.sh`)

A second `exec-once` watcher. It blocks on udev power-supply events and
raises a light notification and sound on each plug or unplug edge. The
sound is skipped while the default sink is muted. It also hands the new
cable state to `powermode.sh`.

## Lock before sleep (`sleep-lock.py`)

The systemd user unit `mango-sleep-lock.service` locks the screen before
suspend and pauses the pomodoro across the sleep. It holds a logind delay
inhibitor and releases it only when `swaylock --ready-fd` confirms the
lock (or after a 3 s bound). It covers every suspend path, lid close
included.

This is deliberately not hypridle's job: on compositors without the
Hyprland lock-notify protocol, hypridle releases its sleep inhibitor as
soon as the lock command is spawned, before the screen is locked.
`hypr/hypridle.conf` carries the full explanation.

`sleep-lock.py test` runs a self-check; `SIGUSR1` on the running daemon
runs a lock-and-pause rehearsal without a suspend.

## Keep-awake

The bar's inhibit pill toggles `mango-keepawake.service`, a
`systemd-inhibit` block on idle and lid-switch handling. logind honors the
lid part only with `LidSwitchIgnoreInhibited=no` — see the checklist.

## Display rescue (`rescue-outputs.sh`)

Symptom: the desktop stops drawing and stays frozen on the last frame.
Input still works (keybinds fire, a password typed into a lock screen still
reaches it) — only painting stops. This is mango's `selmon == NULL` state:
every output got disabled, and with nothing to paint to, the compositor has
no surface to draw. It is not a hang and it is not a crash.

Confirmed 2026-08-26 (source-level, against the exact installed
`mangowm-git` build) that `wlopm --off '*'` (DPMS, `hypridle.conf`'s
600 s screen-off) is **not** the cause: a DPMS-off monitor keeps its real
geometry and stays disable-only, not layout-removed. The remaining
suspects are anything that applies a `zwlr_output_management_v1`
configuration with an output disabled — `wdisplays` is installed and does
exactly this — or a `disable_monitor`/`toggle_monitor` dispatch (no keybind
here uses either).

Recovery needs no compositor restart and loses no applications: mango
keeps a disabled output on its monitor list and re-enabling it is a plain
IPC call. Three layers:

1. **`mango-outputs.service`** polls for the frozen state and re-enables
   automatically, usually within ~15 s.
2. **`SUPER+SHIFT+o`** runs the same rescue immediately — the manual
   escape, and it works even if the watchdog itself is dead, since input
   keeps responding during the freeze.
3. **A TTY** (`Ctrl+Alt+F3`) as the last resort:
   `~/.config/mango/scripts/rescue-outputs.sh`. Do this before reaching for
   `systemctl restart sddm` — that restart is what loses every open
   application.

```
rescue-outputs.sh        # one-shot: check, rescue if frozen
rescue-outputs.sh watch  # poll loop (what the service runs)
rescue-outputs.sh test   # self-check, no compositor needed
```

The two `windowrule=` fractional-size rules that used to crash mango in
this state (`mango.c:1827-1834` dereferences the monitor with no NULL
guard to resolve a fraction) were changed to literal pixel sizes in
`config.conf` — a crash there would have killed every running application
along with the compositor.

## Power attribution (`system/rapl/`)

`sudo system/rapl/install.sh` makes per-domain power visible to the
battery popup:

- A udev rule group-reads the RAPL energy counters for `wheel`. RAPL is
  root-only by default because of the PLATYPUS side channel
  (CVE-2020-8694); on a single-user laptop that risk does not apply.
- A sudoers rule allows argument-less `powertop` (the battery pill's
  right-click).

powertop ranks devices only after a one-off `sudo powertop --calibrate` on
battery. Without the install, the popup degrades to the total draw number.
