# Machine-specific values

No machine-specific value is hardcoded. The values come from three places:

1. `mango/local.conf` is the per-machine compositor config. `config.conf`
   ends with `source=./local.conf`. The installer seeds this file once
   from `local.conf.example`. It never overwrites the file.
2. `mango/powermode.conf` holds every power-mode knob. The repo tracks the
   file, and the comments are in the file. See [power.md](power.md).
3. The environment variables. The list is below.

vpnguard has no config file. It reads the WireGuard profiles to guard, and
their order, live from NetworkManager. The two properties are
`connection.autoconnect-priority` and `connection.autoconnect`. See
`system/vpnguard/README.md`.

## mango/local.conf

| Value | Why it is per-machine |
|---|---|
| `monitorrule=...` | the output name, the resolution and the scale. Find yours with `mmsg get outputs` |
| `env=XDG_DATA_DIRS,...` | mango's `env=` does not expand `$HOME`, so the flatpak export path must be absolute. Without it the KDE app database is empty and "Open With" lists nothing |
| `env=XDG_MENU_PREFIX,plasma-` | it matches the menu file in `/etc/xdg/menus` |
| `env=MANGO_KEEPASS_DB,/path/db.kdbx` | it selects the database that KeePassXC opens at login. There is no default. If it is unset, the login autostart does nothing and you start KeePassXC by hand. You always type the password by hand. This value does not change that |
| `env=MANGO_SCREENSHOT_DIR,/path` | the screenshot target, if it is not `~/Pictures` |

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
| `MANGO_VG_BUDGET` | `90` (seconds) | the dial-chain backstop of `mango-vpnguard auto` |
| `MANGO_VG_STATE_FILE` | `/run/mango-vpnguard/state` | `net.sh`, bard |
| `MANGO_VG_BIN` | `mango-vpnguard` | `net.sh`. It points `list` at a fixture during `--selftest` |
| `MANGO_REMOTE_WG_DEV` | the `ROLE=always` VPN device from the config | `system/remote/install.sh` |

More variables override a path that a helper uses. Each one points the
helper at a different binary, directory or file. The tests set them to
fixtures. The defaults are correct on a normal system.

- Sysfs paths, for tests: `MANGO_CPU_SYS`, `MANGO_RAPL_*`,
  `MANGO_UPOWER_DIR`, `MANGO_BACKLIGHT_DIR`, `MANGO_PM_*`.
- `mango-vpnguard` binaries, each with the plain command name as the
  default: `MANGO_VG_NMCLI`, `MANGO_VG_WG`, `MANGO_VG_IW`, `MANGO_VG_IP`,
  `MANGO_VG_IPTABLES`, `MANGO_VG_IP6TABLES`, `MANGO_VG_UFW`.
- `mango-vpnguard` directories: `MANGO_VG_UFW_ETC` (`/etc/ufw`) and
  `MANGO_VG_RUN_DIR` (`/run/mango-vpnguard`).
- `mango-hotspot`: `MANGO_HS_HOSTAPD` (`hostapd`), `MANGO_HS_DNSMASQ`
  (`dnsmasq`), `MANGO_HS_RUN_DIR` (`/run/mango-hotspot`) and
  `MANGO_HS_SUBNET` (`10.44.0`).
- `nordlynx-import`: `MANGO_NORD_API` (`https://api.nordvpn.com/v1`),
  `MANGO_NORD_ADDR` (`10.5.0.2/16`) and `MANGO_NORD_TOKEN_FILE`
  (`~/.config/mango/nordvpn-token`).
- `MANGO_BAT_DRYRUN` is empty by default. With a value,
  `battery-guard.sh` prints the suspend step instead of doing it.

## Non-mango application preferences

This is not the config of this repo. It is here because mango's window
placement (one window per tag) depends on it. `install-config.sh` does not
manage these files. The app rewrites its own config on exit, so a symlink
into the repo loses the setting at the next close of the app.

| App | File | Value | Why |
|---|---|---|---|
| Master PDF Editor | `~/.config/Code Industry/Master PDF Editor.conf` | `open_one_window=false` | With `true` (the app's own default), a second document opened while a Master PDF Editor window already exists on another tag becomes a tab in that window instead of a new one on the tag you are viewing. Set to `false` on this machine (2026-08-28) so every opened document gets its own window, mapped onto the current tag. Tradeoff: two documents opened on the *same* tag also get separate windows now, instead of tabs. |
