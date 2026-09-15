# Install

## The four steps

`install.sh` runs the four parts in order, and it asks before each one. Then
it prints the manual steps. Each part also runs alone. Each part accepts
`--dry-run`.

| Script | Function | Writes to the system? |
|---|---|---|
| `install-deps.sh` | Reports the missing packages. It shows one `sudo pacman -S` line and one `yay -S` line | No. It is read-only. |
| `install-config.sh` | Symlinks the repo into `~/.config`. Builds `mango-bard`. Links the systemd user units. Seeds the per-machine files. Runs the color pipeline one time | Yes |
| `system/<name>/install.sh` | The root installers, as a menu. You choose them | Yes, as root |
| `install-check.sh` | Verifies the result | No. It is read-only. |

The exit code of `install.sh` is the exit code of the verify step. `0` means
the install checks out.

### Flags

| Flag | Effect |
|---|---|
| `--dry-run` | Print every action. Change nothing. |
| `--yes` | Answer yes to each question. It runs no root installer unless `--system=` names one. |
| `--no` | Answer no to each question. |
| `--system=a,b` | Run those root installers. Show no menu. `--system=all` runs every one. |
| `--check` | Run the verify step alone. |
| `--skip` | Leave every file that already exists. It links only what is absent. |
| `--force` | Replace an existing file with no backup. |

`--skip` and `--force` change the symlink step only. `install.sh` passes them
to `install-config.sh`. The per-machine files (`theme.json`, `local.conf`,
`git/config.local`) hold values that belong to this machine alone, so
`--force` never overwrites them. A root installer accepts neither flag: each
one decides for itself, and each is safe to re-run.

Without a flag, the installer backs up each file that it replaces to
`<file>.bak.<timestamp>`.

## The verify step

`install-check.sh` proves an install. It writes nothing, and it needs no
sudo. It reports `ok`, `warn` or `FAIL` for each check, and it exits `1` when
one check fails. Add `-v` to see the checks that pass.

It checks:

- Every tracked file is a symlink back to this repo. A file that is not a
  link means an edit in the repo does not reach the desktop.
- The per-machine files exist. `git/config.local` has a name and an email.
  `mango/local.conf` has no unexpanded `$HOME`.
- `~/.local/bin/mango-bard` is a real binary, not a symlink, and it is newer
  than its sources. A symlink here is the 2026-09-06 outage: the build
  directory is temporary and is empty after a reboot.
- The five units that mango starts at login are enabled **and** running. The
  four on-demand units are linked and not enabled.
- The kdeconnect autostart and D-Bus overrides are in place.
- The matugen output exists.
- Which root installers landed, from the `# check:` line of each one.
- The parts of the manual checklist below that a script can see.

The expectations live in `install-check.sh` itself, not in
`install-config.sh`. A check that asks the installer what it did can only
agree with it. When the two drift apart, the check must fail and say so.

The config installer obeys these rules:

- The installer links each file one by one. It never links a whole
  directory. Generated files can then stay next to the symlinks. The files
  do not show in `git status`.
- The installer does not overwrite a file without a
  `<file>.bak.<timestamp>` backup.
- A second run is safe. The installer does not touch the files that are
  already linked.
- The installer never uses git. You make the commits as a separate manual
  step.

`install-config.sh` also does these tasks:

- Copies `mango/theme.json.example`, `mango/local.conf.example`, and
  `git/config.local.example` to their live names one time. It never
  overwrites a live copy.
- Caches the DIMM data for the memory popup. It runs
  `sudo dmidecode -t memory` one time and writes `~/.cache/mango-meminfo`.
  To refresh the cache, delete the file. Then run the installer again.
- Aliases the Nextcloud tray icon names into
  `~/.local/share/icons/hicolor`. The Nextcloud client asks for icon names
  that exist only in the Breeze theme. The aliases point to the branded
  icons that the client already supplies.
- Compiles `ironbar/fast-tooltips.c` to `~/.local/lib/mango/fast-tooltips.so`.
  This LD_PRELOAD shim makes the fixed 500 ms GTK3 tooltip delay shorter.
- Builds and installs `mango-bard` with `cargo install`. This puts a real
  binary at `~/.local/bin/mango-bard`. The script does not make a symlink to
  the build output, because that output is not permanent.
  `ironbar/bard/target/` is in `.gitignore`, and `$CARGO_HOME/config.toml`
  sends the build to tmpfs. A symlink to the build output dies at the
  next reboot, and the bar does not start. See [shell.md](shell.md) for that
  file and for the debug-info settings. The script skips the build when
  the installed binary is newer than the sources.
- Links the systemd user units. Enables the five that mango starts at login:
  `mango-bard`, `ironbar`, `mango-sleep-lock`, `mango-powerkey` and
  `mango-outputs`. The other four stay linked and disabled. The bar toggles
  start those when necessary.
- Links a drop-in override for the `arch-update.timer` unit of the
  `arch-update` package. The override sets `Persistent=true` and
  `OnUnitActiveSec=1h`. A missed run then replays at the next boot. The bar
  pill does not stay stale.
- Runs `switchwall.sh --noswitch` to write the matugen output.

## Packages

`install-deps.sh` checks these groups. ᴬ marks AUR packages.

| Group | Packages |
|---|---|
| Core | `mangowm-git`ᴬ `ironbar kitty rofi-wayland mako` `wlogout`ᴬ `swaylock hypridle matugen awww cliphist wl-clipboard` |
| Tools | `grim slurp swappy hyprpicker tesseract tesseract-data-eng wf-recorder brightnessctl playerctl wireplumber networkmanager nm-connection-editor iw blueman pavucontrol-qt jq libnotify libpulse xdg-user-dirs btop dmidecode imagemagick python-gobject wayvnc kdeconnect` |
| Look | `fish starship eza ttf-jetbrains-mono-nerd` `adw-gtk-theme-git`ᴬ `breeze-plus`ᴬ `kde-cli-tools ttf-ibm-plex` `ttf-material-symbols-variable-git`ᴬ |
| Shell | `fd fzf zoxide bat yazi git-delta` |
| Optional | `keepassxc nextcloud-client dolphin` `arch-update`ᴬ |
| Suggested | `tealdeer entr lazygit sd dust duf trash-cli satty udiskie wl-clip-persist` |

Notes:

- `adw-gtk-theme-git` supplies the `adw-gtk3` themes. The wallpaper switch
  changes between them. `breeze-plus` supplies the related icon themes.
- `btop` opens when you click the CPU pill or the memory pill. `dmidecode`
  fills the DIMM cache at install time. The two packages are not mandatory.
  If one is absent, the popup removes the section that it cannot fill.
- The UI font is IBM Plex Sans (`ttf-ibm-plex`).

## Root-level installers (`system/`)

`install-config.sh` writes only in `$HOME`. Each directory in `system/` has
its own root script. The installer never symlinks these directories:

| Directory | Installs | See |
|---|---|---|
| `system/sddm/` | the matugen-themed SDDM greeter and its color sync tool | [theming.md](theming.md) |
| `system/plymouth/` | the plymouth LUKS prompt that matches the greeter, and its asset sync tool | [theming.md](theming.md) |
| `system/tpm-totp/` | the TPM boot-attestation code on the prompt | [theming.md](theming.md) |
| `system/grub/` | the hidden GRUB menu that keeps the firmware logo, and the shared guarded `grub-regen` | [theming.md](theming.md) |
| `system/boot-pin/` | the pinned-kernel and verbose-console GRUB rescue entries | [theming.md](theming.md) |
| `system/secureboot/` | the `mango-sign-boot` tool, the GPG and sbctl keys, the mkinitcpio and pacman hooks, and a staged `GRUB-SB` image. The script signs files. It enrolls no key and it enables no firmware setting | — |
| `system/powermode/` | the root helper that writes the CPU and PCI power knobs | [power.md](power.md) |
| `system/rapl/` | read access to the RAPL power counters, and a powertop sudo rule | [power.md](power.md) |
| `system/fprint-notify/` | the root helper and the icon that show a notification for each fingerprint request. The script prints the two PAM lines, but it does not edit `/etc/pam.d/sudo` | [security.md](security.md) |
| `system/vault/` | the udev rule that gives the active seat an ACL on the Power Button evdev node. `mango-powerkey` then works after you leave the `input` group. Also holds `vault-session`, the wallet sandbox launcher, which `install-config.sh` links to `~/.local/bin/` | — |
| `system/hotspot/` | the Wi-Fi hotspot helper | — |
| `system/remote/` | the remote access units (wayvnc, KDE Connect) | — |
| `system/i915/` | the GPU compute timeout udev rules | — |
| `system/memtune/` | the zram size, the swap readahead, the dirty-page caps, and the sysfs write that turns zswap off. zswap runs in front of zram by default and compresses each page before zram sees it | — |
| `system/libvirt-net/` | the libvirt network config | its own README |
| `system/vpnguard/` | the fail-closed egress and the WireGuard failover. The membership and the order come from the `connection.autoconnect-priority` property of NetworkManager. There is no config file and no hardcoded VPN | its own README |

Run each script one time. Run it again after you change a file in its
directory. Use the menu in `install.sh`, or call one directly:

```bash
sudo system/<name>/install.sh
```

### What each installer declares

Every `system/<name>/install.sh` carries two declarations in its own header.
`install.sh` and `install-check.sh` read them, so a new installer joins the
menu and the verify step with no edit to either:

| Line | Meaning |
|---|---|
| `# check: <path>` | The file that proves the installer landed. `install-check.sh` tests it. |
| `# check: ufw:<text>` | The same, for a ufw rule. The check greps `/etc/ufw/user.rules` for that comment, and it reports `?` when it cannot read the file. |
| `# risk: boot` | The installer changes what boots, or how you log in. The menu marks it `!` and asks a second time. |

The menu text for each installer is the first sentence of its header
comment. Keep that first sentence short and true.

Five installers carry `# risk: boot`: `grub`, `plymouth`, `sddm`,
`secureboot` and `tpm-totp`. A bad result from one of these can leave the
machine with no boot and no login screen.

## Manual checklist

This desktop needs system state that no tracked file contains. Do these
steps after the first install:

- [ ] In `/etc/systemd/logind.conf.d/10-power.conf`, set
      `HandlePowerKey=ignore` and `HandlePowerKeyLongPress=ignore`.
      `powerkey.py` then controls the power button. Set
      `LidSwitchIgnoreInhibited=no`. The keep-awake toggle of the bar can
      then block the lid switch. See [power.md](power.md).
- [ ] In `/etc/UPower/UPower.conf`, keep `PercentageAction=2.0` and
      `CriticalPowerAction=Auto`. These settings are the last resort below
      the battery guard.
- [ ] In KeePassXC, add the attribute `application=mango` to the OpenRouter
      key entry. Without this attribute, the AI chat (`Alt+I`) cannot find
      the key.
- [ ] Delete `/usr/local/bin/keepassxc-stash-pw` to remove the retired
      KeePassXC auto-unlock. Also delete the `pam_exec.so expose_authtok`
      line in `/etc/pam.d/sddm` that fills `/run/keepassxc-unlock/$USER`.
      `keepassxc-autounlock.sh` no longer reads that stash. You now type the
      password into the usual KeePassXC prompt at login. In
      `~/.config/keepassxc/keepassxc.ini`, set `MinimizeOnStartup=false`
      under `[GUI]` and `MinimizeAfterUnlock=true` under `[General]`. The
      prompt is then visible, and the window hides itself after you unlock
      it.
- [ ] After you run `system/fprint-notify/install.sh`, edit
      `/etc/pam.d/sudo`. Add
      `auth optional pam_exec.so quiet /usr/local/bin/mango-fprint-notify`
      above the `auth sufficient pam_fprintd.so` line. The installer prints
      this line, but it never edits the file. See
      [security.md](security.md).
- [ ] Mask the GNOME keyring user units. Link
      `gnome-keyring-daemon.{service,socket}` to `/dev/null`. KeePassXC then
      controls the Secret Service.
- [ ] Install `git-delta` before you delete `~/.gitconfig`. If `git-delta`
      is absent, each paged git command fails. Git reads the XDG config
      first and `~/.gitconfig` second. The later file wins. The tracked
      config does nothing while `~/.gitconfig` exists.
- [ ] Put your name and your email in `~/.config/git/config.local`. Git does
      not commit without an identity.
- [ ] Put a wallpaper in `~/Wallpapers/`.
- [ ] `arch-update --tray` does not start. The arch-update pill of the bar
      shows the same data. The `arch-update` package is still necessary for
      its check binary and its timer.
- [ ] Hibernate does not work on this machine. The swapfile is smaller than
      the RAM, and the kernel command line has no `resume=` parameter. The
      wlogout Hibernate button does nothing, and the battery guard does a
      suspend instead. To enable hibernate, make a swapfile larger than the
      RAM. Then add `resume=` and `resume_offset=` to the kernel command
      line. Add the `resume` hook to `mkinitcpio.conf`.
- [ ] Two OOM daemons run on this machine: `systemd-oomd` and `nohang`. Keep
      `nohang` and stop the other one. `systemd-oomd` kills a full cgroup.
      Under `user.slice` that cgroup is the graphical session. `nohang` kills
      one process, which is the correct result here. Run
      `sudo systemctl disable --now systemd-oomd.service`. Then set
      `zram_checking_enabled = True` in `/etc/nohang/nohang.conf` and restart
      `nohang`. The stock file sets `False`, so `nohang` does not look at
      zram. Almost all swap on this machine is zram. The
      `soft_threshold_max_zram` and `hard_threshold_max_zram` limits in that
      same file stay unused while the flag is `False`.
- [ ] These files are not tracked yet: `$CARGO_HOME/config.toml` (see
      [shell.md](shell.md)), `~/.config/autostart/`,
      `~/.config/environment.d/`, `~/.config/mimeapps.list`, the custom
      `.desktop` files, the KDE app rc files, `{chrome,code}-flags.conf`,
      and `Code/User/settings.json`.
