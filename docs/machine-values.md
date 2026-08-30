# Machine-specific values

Nothing machine-specific is hardcoded. Values come from three places:

1. `mango/local.conf` — per-machine compositor config. `config.conf` ends
   with `source=./local.conf`. The installer seeds it once from
   `local.conf.example` and never overwrites it.
2. `mango/powermode.conf` — every power-mode knob, tracked and commented in
   place. See [power.md](power.md).
3. Environment variables, listed below.

vpnguard has no config file: which WireGuard profiles it guards, and in
what order, is read live from NetworkManager's own
`connection.autoconnect-priority` and `connection.autoconnect`. See
`system/vpnguard/README.md`.

## mango/local.conf

| Value | Why it is per-machine |
|---|---|
| `monitorrule=...` | output name, resolution, scale — find yours with `mmsg get outputs` |
| `env=XDG_DATA_DIRS,...` | mango's `env=` does not expand `$HOME`, so the flatpak export path must be absolute. Without it the KDE app database is empty and "Open With" lists nothing |
| `env=XDG_MENU_PREFIX,plasma-` | matches the menu file present in `/etc/xdg/menus` |
| `env=MANGO_KEEPASS_DB,/path/db.kdbx` | selects which database KeePassXC opens at login. No default — unset means the login autostart does nothing and you start KeePassXC by hand. You always type the password by hand; this value does not control that |
| `env=MANGO_SCREENSHOT_DIR,/path` | screenshot target, if not `~/Pictures` |

## Environment variables

| Variable | Default | Read by |
|---|---|---|
| `MANGO_WIFI_DEV` | `wlo1` | `net.sh`, `wifi-menu.sh`, `hotspot.sh`, bard |
| `MANGO_ETH_DEV` | `eno2` | `net.sh`, `eth-toggle.sh`, bard |
| `MANGO_HS_IFACE` | `p2p0` | hotspot scripts, bard |
| `MANGO_DOCKER` | `docker` | `docker-menu.sh`, bard |
| `MANGO_WORLD_TZ` | `America/New_York,Europe/Prague,Asia/Bangkok,Indian/Mauritius` | clock popup |
| `MANGO_CALENDAR_CMD` / `_APPID` | `thunderbird -calendar` / `org.mozilla.Thunderbird` | `clock.sh --calendar` |
| `MANGO_POMODORO` | `25,5,15,4` (work, short, long, cycles, minutes) | bard |
| `MANGO_POMODORO_MUTE` | `30` (minutes) | bard |
| `MANGO_DMI_CACHE` | `~/.cache/mango-meminfo` | memory popup |
| `MANGO_BAT_DIR` / `MANGO_AC_DIR` | auto-discovered | bard, power scripts |
| `MANGO_WEAR_STATE` / `MANGO_WEAR_REPLACE` | state file / `80` | battery wear history |
| `MANGO_BATTERY` | `20,10,5,3` (percent tiers) | `battery-guard.sh` |
| `MANGO_POWERMODE_CONF` | `~/.config/mango/powermode.conf` | `powermode.sh` |
| `MANGO_SCREENSHOT_DIR` | `~/Pictures/Screenshots` | `screenshot.sh` |
| `MANGO_POWERMENU_SIZE` | `760x430` | `powermenu.sh` |
| `MANGO_DOWNLOAD_DIR` / `_FRESH_MIN` | `~/Downloads` / `5` | `busy.sh` |
| `MANGO_PACMAN_LCK` | `/var/lib/pacman/db.lck` | `busy.sh` |
| `MANGO_AI_DIR` / `MANGO_AI_MODEL` | `~/.local/share/rofi-ai` / a free OpenRouter model | `rofi/ai.sh` |
| `MANGO_TOOLTIP_DELAY_MS` | `80` | the GTK tooltip shim |
| `MANGO_VG_BUDGET` | `90` (seconds) | `mango-vpnguard auto`'s dial-chain backstop |
| `MANGO_VG_STATE_FILE` | `/run/mango-vpnguard/state` | `net.sh`, bard |
| `MANGO_VG_BIN` | `mango-vpnguard` | `net.sh` (points `list` at a fixture during `--selftest`) |
| `MANGO_REMOTE_WG_DEV` | the config's `ROLE=always` VPN device | `system/remote/install.sh` |

Sysfs path overrides exist for tests (`MANGO_CPU_SYS`, `MANGO_RAPL_*`,
`MANGO_UPOWER_DIR`, `MANGO_BACKLIGHT_DIR`, `MANGO_PM_*`); the defaults are
correct on a normal system.

## Non-mango application preferences

Not this repo's config, but noted here because mango's window placement
(one window per tag) depends on them. `install-config.sh` does not manage
these files — the app rewrites its own config on exit, so symlinking it
into the repo would just lose the setting on the app's next close.

| App | File | Value | Why |
|---|---|---|---|
| Master PDF Editor | `~/.config/Code Industry/Master PDF Editor.conf` | `open_one_window=false` | With `true` (the app's own default), a second document opened while a Master PDF Editor window already exists on another tag becomes a tab in that window instead of a new one on the tag you are viewing. Set to `false` on this machine (2026-08-28) so every opened document gets its own window, mapped onto the current tag. Tradeoff: two documents opened on the *same* tag also get separate windows now, instead of tabs. |
