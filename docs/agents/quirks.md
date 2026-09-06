# mango-dotfiles — project quirks

This file is the source of truth for this machine's desktop. The desktop is
mango (a Wayland compositor), ironbar with the mango-bard daemon, kitty,
rofi, the matugen colour pipeline, wlogout, fish and fontconfig.

Read this file before you change any of the areas listed below.

## The rule

**Any system or desktop change must land in this repo, in the same session.**
Do not plan to copy it across later. The repo *is* the config. A change that
exists only in `~/.config` disappears at the next `install.sh`. It
disappears silently.

`install.sh` symlinks each tracked file into `~/.config` one by one. An edit
through a symlink thus edits the repo. That works, but three actions detach a
file with no warning:

- You create a **new** file in `~/.config`. Nothing links it back.
- You `mv` a file over a symlink. This replaces the link with a regular file.
- An application rewrites a file in place of an edit in place.

Check that an edit landed, before you trust it:

```bash
ls -l ~/.config/<file>        # must be a symlink into ~/src/mango-dotfiles
```

Run this check after any session that changed the desktop:

```bash
git -C ~/src/mango-dotfiles status --short
```

An empty result means you changed nothing. Look again if you think you did.
An untracked file means something was created outside the repo. Move that
file into the repo, and add it to `install.sh`.

## Never edit these — they are matugen output

An edit holds until the next wallpaper switch. The switch then reverts it,
silently:

```
~/.config/ironbar/style.css         ~/.config/mango/colors.conf
~/.config/ironbar/config.json       ~/.config/mango/keybinds.html
~/.config/kitty/theme.conf          ~/.config/rofi/colors.rasi
~/.config/swaylock/config           ~/.config/gtk-{3,4}.0/gtk.css
~/.config/mako/config               ~/.config/kdeglobals
~/.config/wlogout/style.css
/usr/share/sddm/themes/mango-sddm/Colors.qml
/usr/share/sddm/themes/mango-sddm/background.*
/boot/mango-plymouth.img
```

`ironbar/style.css` comes from `matugen/templates/ironbar/style.css`.
`ironbar/config.json` comes from `mango-bard gen-config`; edit
`ironbar/bard/src/genconfig.rs` for that file. `mango/keybinds.html` comes
from `mango/scripts/keybinds-cheatsheet.py`, not from matugen. For the rest,
edit `matugen/templates/<thing>`. Then regenerate them:

```bash
~/.config/mango/scripts/switchwall.sh --noswitch
pkill -x ironbar; ~/.config/ironbar/scripts/start.sh &   # if you touched the bar
```

The per-file symlink is what keeps `git status` clean. matugen writes each
file into `~/.config/<app>/`, beside the symlink, and never into the repo.
`.gitignore` lists most of these paths as a second line of defence. It does
not list `ironbar/style.css`, `ironbar/config.json`, `gtk-3.0/gtk.css`,
`gtk-4.0/gtk.css` or `kdeglobals`. If `git status` is dirty immediately after
a theme switch, the split between tracked files and generated files has
broken.

The same rule applies to two files that change at runtime. `switchwall.sh`
writes `wallpaperPath` and `accentColor` into `mango/theme.json`.
`mango/local.conf` holds per-machine values. Track the `.example` version of
each file, never the live one.

## System state this repo does *not* cover

A change to any of these is still a system change. Record it here. If the
repo cannot track it as a file, record it in the docs/install.md checklist:

- `hypridle.conf`, and anything else under `~/.config/hypr/`
- `/usr/local/bin/keepassxc-stash-pw` and the SDDM PAM hook that feeds
  `/run/keepassxc-unlock/$USER` — retired. `keepassxc-autounlock.sh` no
  longer reads the stash. Remove both by hand (see docs/install.md)
- the AUR `keepassxc-unlock` package (`keepassxc-login-monitor.service`,
  `keepassxc-unlock@.service`) — also retired, removed 2026-09-01. It tried
  to open the database over DBus with a TPM-sealed credential. The TPM key
  did not match, thus the call always failed. That failed `openDatabase`
  call raised KeePassXC's separate "Unlock Database" window next to the main
  window at every login. That is the second-window-at-startup symptom. You
  type the password by hand. This machine has no auto-unlock
- `~/.config/keepassxc/keepassxc.ini` — `MinimizeOnStartup=false` shows the
  unlock prompt at login. `MinimizeAfterUnlock=true` hides the window after
  you unlock it. `ShowTrayIcon` and `MinimizeToTray` must stay false. The
  `windowrule` in `mango/config.conf` matches on the appid only, not on the
  title. Mango applies a window rule once, at map time. The main window's
  title is still "[Locked]" at that time, thus a rule anchored on the title
  never matches
- `systemd --user` masks for `gnome-keyring-daemon.{service,socket}` →
  `/dev/null`. These masks let KeePassXC own the Secret Service
- `~/.config/autostart/`, `~/.config/environment.d/`, `~/.config/mimeapps.list`
- the login screen's *installed* side: `/usr/local/bin/sddm-theme-sync`,
  `/usr/local/share/sddm-theme-sync/Colors.qml.in`,
  `/etc/sudoers.d/sddm-theme-sync`, `/etc/sddm.conf`, and the theme in
  `/usr/share/sddm/themes/mango-sddm/`. `system/sddm/` tracks the *sources*
  for all of these. `sudo system/sddm/install.sh` pushes them out. An edit to
  an installed copy is the same mistake as an edit to matugen output
- the plymouth prompt's *installed* side: `/usr/share/plymouth/themes/mango/`
  (never with a `dyn/` inside), `/usr/local/bin/plymouth-theme-sync`,
  `/etc/sudoers.d/plymouth-theme-sync`, `/etc/plymouth/plymouthd.conf`,
  `/etc/systemd/system/sddm.service.d/10-mango.conf`
  (After=plymouth-quit-wait; never a `plymouth quit --retain-splash`
  drop-in, which hangs X under SDDM on i915), the `plymouth` hook in
  `HOOKS`, `splash` and the second image in `GRUB_EARLY_INITRD_LINUX_CUSTOM`
  in `/etc/default/grub`. `system/plymouth/` holds the sources.
  `sudo system/plymouth/install.sh` pushes them out, and runs
  `mkinitcpio -P` and `grub-regen`
- the attestation code's *installed* side: `/usr/local/bin/mango-totp`,
  `/usr/local/share/mango-totp/mango-totp.service`,
  `/usr/lib/initcpio/install/mango-totp`, the `mango-totp` hook in `HOOKS`,
  and the secret in the TPM's NV index (never a file). `system/tpm-totp/`
  holds the sources. The seal does not maintain itself. Every `linux`
  upgrade moves PCR 4, because firmware LoadImage measures the kernel under
  Secure Boot. The next boot then needs
  `sudo tpm2-totp -P - -p 0,2,4,7 reseal`. Never run `generate`, which mints
  a new secret
- GRUB's *installed* side beyond the above: `/usr/local/bin/grub-regen`,
  `GRUB_TIMEOUT_STYLE`/`GRUB_TIMEOUT`/`GRUB_TERMINAL_OUTPUT`/`GRUB_BACKGROUND`
  in `/etc/default/grub` and `/boot/grub/mango-logo.png` (`system/grub/`,
  always the `--gfx` variant, because the EFI text console is blank on this
  firmware), the `# BEGIN mango-pin` block in `/etc/grub.d/40_custom` and
  `/boot/pinned/` (`system/boot-pin/`; never `/boot/*-pinned*`, because
  `10_linux` globs `/boot/vmlinuz-*` and would make the pinned copy the
  default entry)
- battery power attribution: `/etc/udev/rules.d/mango-rapl.rules` (it gives
  the `wheel` group read access to RAPL's `energy_uj`) and
  `/etc/sudoers.d/mango-powertop` (NOPASSWD, `powertop` with no argument).
  `system/rapl/` tracks the sources. `sudo system/rapl/install.sh` pushes
  them out
- power modes: `/usr/local/bin/mango-powermode` (the only program that
  writes CPU EPP, turbo, platform profile and PCI runtime PM) and
  `/etc/sudoers.d/mango-powermode` (NOPASSWD, fixed `apply` verb, and the
  script validates the payload again). `system/powermode/` tracks the
  sources. `sudo system/powermode/install.sh` pushes them out. The repo DOES
  track and symlink the values themselves (`mango/powermode.conf`), like any
  other file. Only the root helper needs this install step
- KeePassXC entry attributes — `rofi/ai.sh` looks up `application=mango`

## Conventions

- Scripts are POSIX-ish bash with `set -u`. A non-trivial script carries a
  `test` subcommand as a self-check (`wait-for-keepass-unlock.sh test`,
  `keyring-lookup.sh test`).
- Put a machine-specific value in `mango/local.conf`. Never hardcode it. See
  the docs/machine-values.md table.
- Attribution for ported work stays in the README Credits section. A change
  of a path name does not change the provenance.
