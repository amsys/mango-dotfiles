# Install

## The two installers

`install.sh` runs both parts in order, then prints the manual steps.
Each part also runs alone, and each accepts `--dry-run`.

| Script | Function | Writes to the system? |
|---|---|---|
| `install-deps.sh` | Reports missing packages, split into a `sudo pacman -S` line and a `yay -S` line | No. It is read-only. |
| `install-config.sh` | Symlinks the repo into `~/.config`, builds `mango-bard`, links systemd user units, seeds per-machine files, runs the color pipeline once | Yes |

Rules the config installer follows:

- It links each file one by one, never a whole directory. Generated files can
  then sit next to the symlinks without appearing in `git status`.
- It does not overwrite a file without a `<file>.bak.<timestamp>` backup.
- A second run is safe. Files that are already linked are not touched.
- It never touches git. Commits stay a separate, manual step.

Beyond the symlinks, `install-config.sh` also:

- Copies `mango/theme.json.example`, `mango/local.conf.example`, and
  `git/config.local.example` to their live names, once. It never overwrites
  the live copies.
- Caches DIMM data for the memory popup: it runs `sudo dmidecode -t memory`
  once and writes `~/.cache/mango-meminfo`. Delete the file and run the
  installer again to refresh it.
- Aliases the Nextcloud tray icon names into
  `~/.local/share/icons/hicolor`. The Nextcloud client requests icon names
  that exist only in the Breeze theme. The aliases point at the branded
  icons the client already ships.
- Compiles `ironbar/fast-tooltips.c` to `~/.local/lib/mango/fast-tooltips.so`.
  This LD_PRELOAD shim shortens GTK3's fixed 500 ms tooltip delay.
- Builds `mango-bard` with `cargo build --release` and links the binary to
  `~/.local/bin/mango-bard`. The build is skipped when the binary is newer
  than the sources.
- Links the systemd user units and enables `mango-bard.service` and
  `mango-sleep-lock.service`. The other units stay disabled; bar toggles
  start them on demand.
- Runs `switchwall.sh --noswitch` to materialize the matugen output.

## Packages

`install-deps.sh` checks these groups. ᴬ marks AUR packages.

| Group | Packages |
|---|---|
| Core | `mangowm-git`ᴬ `ironbar kitty rofi-wayland mako` `wlogout`ᴬ `swaylock hypridle matugen swaybg cliphist wl-clipboard` |
| Tools | `grim slurp swappy hyprpicker tesseract tesseract-data-eng wf-recorder brightnessctl playerctl wireplumber networkmanager nm-connection-editor iw blueman pavucontrol-qt jq libnotify libpulse xdg-user-dirs atop btop dmidecode imagemagick python-gobject wayvnc kdeconnect` |
| Look | `fish starship eza ttf-jetbrains-mono-nerd` `adw-gtk-theme-git`ᴬ `breeze-plus`ᴬ `kde-cli-tools ttf-ibm-plex` `ttf-material-symbols-variable-git`ᴬ |
| Shell | `fd fzf zoxide bat yazi git-delta` |
| Optional | `keepassxc nextcloud-client dolphin` `arch-update`ᴬ |
| Suggested | `tealdeer entr lazygit sd dust duf trash-cli satty udiskie wl-clip-persist` |

Notes:

- `adw-gtk-theme-git` provides the `adw-gtk3` themes the wallpaper switch
  toggles between. `breeze-plus` provides the matching icon themes.
- `atop` feeds the CPU popup's "recent peaks" list. Its service must run
  (see the checklist). `btop` opens when you click the CPU or memory pill.
  `dmidecode` fills the DIMM cache at install time. All three degrade
  quietly — the popup drops the section it cannot fill.
- The UI font is IBM Plex Sans (`ttf-ibm-plex`).

## Root-level installers (`system/`)

`install-config.sh` only touches `$HOME`. Everything under `system/` installs
with its own root script and is never symlinked:

| Directory | Installs | See |
|---|---|---|
| `system/sddm/` | the matugen-themed SDDM greeter and its color sync tool | [theming.md](theming.md) |
| `system/cryptbox/` | the boot-time LUKS prompt frame and its palette sync tool | [theming.md](theming.md) |
| `system/powermode/` | the root helper that writes CPU/PCI power knobs | [power.md](power.md) |
| `system/rapl/` | read access to RAPL power counters, plus a powertop sudo rule | [power.md](power.md) |
| `system/hotspot/` | the Wi-Fi hotspot helper | — |
| `system/remote/` | remote access units (wayvnc, KDE Connect) | — |
| `system/i915/` | GPU compute timeout udev rules | — |
| `system/libvirt-net/` | libvirt network config | its own README |
| `system/vpnguard/` | fail-closed egress + WireGuard failover — membership and order come from NetworkManager's own `connection.autoconnect-priority`, no config file, no VPN hardcoded | its own README |

Run each once, and again after you change a file in its directory:

```bash
sudo system/<name>/install.sh
```

## Manual checklist

This desktop depends on system state that no tracked file covers. Complete
these steps after the first install:

- [ ] `systemctl enable --now atop.service atopacct.service` — the CPU
      popup's "recent peaks" section reads `/var/log/atop/`.
- [ ] `/etc/systemd/logind.conf.d/10-power.conf` — set
      `HandlePowerKey=ignore` and `HandlePowerKeyLongPress=ignore` so
      `powerkey.py` owns the power button. Set
      `LidSwitchIgnoreInhibited=no` so the bar's keep-awake toggle can block
      the lid switch. See [power.md](power.md).
- [ ] `/etc/UPower/UPower.conf` — keep `PercentageAction=2.0` with
      `CriticalPowerAction=Auto` as the last resort below the battery guard.
- [ ] KeePassXC: add the attribute `application=mango` to the OpenRouter key
      entry. The AI chat (`Alt+I`) cannot find the key without it.
- [ ] `/usr/local/bin/keepassxc-stash-pw` and the SDDM PAM hook that feeds
      `/run/keepassxc-unlock/$USER` (used by `keepassxc-autounlock.sh`).
- [ ] Mask the GNOME keyring user units
      (`gnome-keyring-daemon.{service,socket}` → `/dev/null`) so KeePassXC
      owns the Secret Service.
- [ ] Delete `~/.gitconfig` after you install `git-delta`. Git reads the XDG
      config first and `~/.gitconfig` second, and the later file wins — the
      tracked config does nothing while `~/.gitconfig` exists. Install delta
      first, or every paged git command fails.
- [ ] Fill in `~/.config/git/config.local` (name and email). Git refuses to
      commit without an identity.
- [ ] Put a wallpaper in `~/Wallpapers/`.
- [ ] Hibernate does not work on this machine: the swapfile is smaller than
      RAM and there is no `resume=` on the kernel command line. The wlogout
      Hibernate button is dead, and the battery guard suspends instead. To
      enable it: a swapfile larger than RAM, `resume=`/`resume_offset=`, and
      the `resume` hook in `mkinitcpio.conf`.
- [ ] Not yet tracked: `~/.config/autostart/`, `~/.config/environment.d/`,
      `~/.config/mimeapps.list`, custom `.desktop` files, KDE app rc files,
      `{chrome,code}-flags.conf`, `Code/User/settings.json`.
