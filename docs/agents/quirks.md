# mango-dotfiles — project quirks

Source of truth for this machine's desktop: mango (Wayland compositor),
ironbar + the mango-bard daemon, kitty, rofi, matugen colour pipeline,
wlogout, fish, fontconfig.

Read this file before touching any of the areas listed below.

## The rule

**Any system or desktop change must land in this repo, in the same session.**
Not "later", not "I'll copy it across" — the repo *is* the config. A change
that only exists in `~/.config` is a change that disappears on the next
`install.sh`, and silently.

`install.sh` symlinks every tracked file individually into `~/.config`, so
editing through a symlink edits the repo. That works — but three things detach
a file without warning:

- creating a **new** file in `~/.config` (nothing links it back)
- `mv`-ing a file over a symlink (replaces the link with a regular file)
- an app that rewrites rather than edits in place

So before assuming an edit landed:

```bash
ls -l ~/.config/<file>        # must be a symlink into ~/src/mango-dotfiles
```

And after any session that touched the desktop:

```bash
git -C ~/src/mango-dotfiles status --short
```

Empty means you changed nothing (suspicious if you did). Untracked files mean
something was created outside the repo and needs moving in and adding to
`install.sh`.

## Never edit these — they are matugen output

Editing them works until the next wallpaper switch, then silently reverts:

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

`ironbar/style.css` comes from `matugen/templates/ironbar/style.css`;
`ironbar/config.json` comes from `mango-bard gen-config` (edit
`ironbar/bard/src/genconfig.rs`). For the rest, edit
`matugen/templates/<thing>`. Then regenerate:

```bash
~/.config/mango/scripts/switchwall.sh --noswitch
pkill -x ironbar; ~/.config/ironbar/scripts/start.sh &   # if you touched the bar
```

`.gitignore` lists all of them as a safety net — if `git status` is dirty right
after a theme switch, the tracked-vs-generated split has broken.

Same applies to two runtime-mutable files: `mango/theme.json` (switchwall.sh
writes `wallpaperPath` and `accentColor` into it) and `mango/local.conf`
(per-machine). Track the `.example` versions, never the live ones.

## System state this repo does *not* cover

Changing any of these is still a system change and still needs recording here —
in the docs/install.md checklist if it cannot be tracked as a file:

- `hypridle.conf`, and anything else under `~/.config/hypr/`
- `/usr/local/bin/keepassxc-stash-pw` + the SDDM PAM hook feeding
  `/run/keepassxc-unlock/$USER` — retired. `keepassxc-autounlock.sh` no longer
  reads the stash, so both must be removed by hand (see docs/install.md)
- the AUR `keepassxc-unlock` package (`keepassxc-login-monitor.service`,
  `keepassxc-unlock@.service`) — also retired, removed 2026-09-01. It tried
  to open the database over DBus with a TPM-sealed credential; the TPM key
  did not match, so it always failed and its failed `openDatabase` call
  raised KeePassXC's separate "Unlock Database" window alongside the main
  window at every login — the second-window-at-startup symptom. You type
  the password by hand; there is no auto-unlock on this machine
- `~/.config/keepassxc/keepassxc.ini` — `MinimizeOnStartup=false` shows the
  unlock prompt at login, `MinimizeAfterUnlock=true` hides the window after
  you unlock it. `ShowTrayIcon` and `MinimizeToTray` must stay false. The
  `windowrule` in `mango/config.conf` matches on appid only, not title: mango
  applies window rules once at map, and the main window's title is still
  "[Locked]" at that point, so a title-anchored rule never matches
- `systemd --user` masks for `gnome-keyring-daemon.{service,socket}` → `/dev/null`
  (this is what lets KeePassXC own the Secret Service)
- `~/.config/autostart/`, `~/.config/environment.d/`, `~/.config/mimeapps.list`
- the login screen's *installed* side: `/usr/local/bin/sddm-theme-sync`,
  `/usr/local/share/sddm-theme-sync/Colors.qml.in`,
  `/etc/sudoers.d/sddm-theme-sync`, `/etc/sddm.conf`, and the theme in
  `/usr/share/sddm/themes/mango-sddm/`. The *sources* for all of these are
  tracked in `system/sddm/` — `sudo system/sddm/install.sh` is what pushes them
  out. Editing the installed copies directly is the same mistake as editing
  matugen output
- the plymouth prompt's *installed* side: `/usr/share/plymouth/themes/mango/`
  (never with a `dyn/` inside), `/usr/local/bin/plymouth-theme-sync`,
  `/etc/sudoers.d/plymouth-theme-sync`, `/etc/plymouth/plymouthd.conf`,
  `/etc/systemd/system/sddm.service.d/10-mango.conf` (After=plymouth-quit-wait;
  never a `plymouth quit --retain-splash` drop-in — that hangs X under SDDM
  on i915), the `plymouth` hook in `HOOKS`, `splash` and the second image in `GRUB_EARLY_INITRD_LINUX_CUSTOM`
  in `/etc/default/grub`. Sources in `system/plymouth/`; `sudo
  system/plymouth/install.sh` pushes them out (runs `mkinitcpio -P` and
  `grub-regen`)
- the attestation code's *installed* side: `/usr/local/bin/mango-totp`,
  `/usr/local/share/mango-totp/mango-totp.service`,
  `/usr/lib/initcpio/install/mango-totp`, the `mango-totp` hook in `HOOKS`,
  and the secret in the TPM's NV index (never a file). Sources in
  `system/tpm-totp/`
- GRUB's *installed* side beyond the above: `/usr/local/bin/grub-regen`,
  `GRUB_TIMEOUT_STYLE`/`GRUB_TIMEOUT`/`GRUB_TERMINAL_OUTPUT`/`GRUB_BACKGROUND`
  in `/etc/default/grub` and `/boot/grub/mango-logo.png` (`system/grub/`,
  always the `--gfx` variant: the EFI text console is blank on this
  firmware), the `# BEGIN mango-pin` block in `/etc/grub.d/40_custom` and
  `/boot/pinned/` (`system/boot-pin/`; never `/boot/*-pinned*` — `10_linux`
  globs `/boot/vmlinuz-*` and would make the pinned copy the default entry)
- battery power attribution: `/etc/udev/rules.d/mango-rapl.rules` (group-reads
  RAPL's `energy_uj` for `wheel`) and `/etc/sudoers.d/mango-powertop`
  (NOPASSWD, arg-less `powertop`). Sources tracked in `system/rapl/` —
  `sudo system/rapl/install.sh` pushes them out
- power modes: `/usr/local/bin/mango-powermode` (the only thing that writes
  CPU EPP/turbo/platform-profile/PCI runtime PM) and
  `/etc/sudoers.d/mango-powermode` (NOPASSWD, fixed `apply` verb, payload
  re-validated inside the script). Sources tracked in `system/powermode/` —
  `sudo system/powermode/install.sh` pushes them out. The values themselves
  (`mango/powermode.conf`) ARE tracked and symlinked like any other file —
  only the root helper needs this install step
- KeePassXC entry attributes — `rofi/ai.sh` looks up `application=mango`

## Conventions

- Scripts are POSIX-ish bash with `set -u`; non-trivial ones carry a `test`
  subcommand self-check (`wait-for-keepass-unlock.sh test`,
  `keyring-lookup.sh test`).
- Machine-specific values go in `mango/local.conf`, never hardcoded — see the
  docs/machine-values.md table.
- Attribution for ported work stays in the README Credits section. Renaming
  paths does not change provenance.