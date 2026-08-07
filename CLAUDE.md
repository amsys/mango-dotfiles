# mango-dotfiles

Source of truth for this machine's desktop: mango (Wayland compositor), waybar,
kitty, rofi, matugen colour pipeline, wlogout, fish, fontconfig.

## The rule

**Any system or desktop change made with Claude must land in this repo, in the
same session.** Not "later", not "I'll copy it across" — the repo *is* the
config. A change that only exists in `~/.config` is a change that disappears on
the next `install.sh`, and silently.

`install.sh` symlinks every tracked file individually into `~/.config`, so
editing through a symlink edits the repo. That works — but three things detach a
file without warning:

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
something was created outside the repo and needs moving in + adding to
`install.sh`.

## Never edit these — they are matugen output

Editing them works until the next wallpaper switch, then silently reverts:

```
~/.config/waybar/style.css          ~/.config/mango/colors.conf
~/.config/waybar/scripts/claudebar.sh   ~/.config/mango/keybinds.html
~/.config/kitty/theme.conf          ~/.config/rofi/colors.rasi
~/.config/swaylock/config           ~/.config/gtk-{3,4}.0/gtk.css
~/.config/mako/config               ~/.config/kdeglobals
/usr/share/sddm/themes/mango-sddm/Colors.qml
/usr/share/sddm/themes/mango-sddm/background.*
```

Edit `matugen/templates/<thing>` instead, then regenerate:

```bash
~/.config/mango/scripts/switchwall.sh --noswitch
pkill -SIGUSR2 waybar      # if you touched the bar
```

`.gitignore` lists all of them as a safety net — if `git status` is dirty right
after a theme switch, the tracked-vs-generated split has broken.

Same applies to two runtime-mutable files: `mango/theme.json` (switchwall.sh
writes `wallpaperPath` and `accentColor` into it) and `mango/local.conf`
(per-machine). Track the `.example` versions, never the live ones.

## System state this repo does *not* cover

Changing any of these is still a system change and still needs recording here —
in the README checklist if it can't be tracked as a file:

- `hypridle.conf`, and anything else under `~/.config/hypr/`
- `/usr/local/bin/keepassxc-stash-pw` + the SDDM PAM hook feeding
  `/run/keepassxc-unlock/$USER`
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
- battery power attribution: `/etc/udev/rules.d/mango-rapl.rules` (group-reads
  RAPL's `energy_uj` for `wheel`) and `/etc/sudoers.d/mango-powertop`
  (NOPASSWD, arg-less `powertop`). Sources tracked in `system/rapl/` —
  `sudo system/rapl/install.sh` pushes them out
- KeePassXC entry attributes — `rofi/ai.sh` looks up `application=mango`

## Conventions

- Scripts are POSIX-ish bash with `set -u`; non-trivial ones carry a `test`
  subcommand self-check (`wait-for-keepass-unlock.sh test`,
  `keyring-lookup.sh test`).
- Machine-specific values go in `mango/local.conf`, never hardcoded — see the
  README's "Machine-specific values" table.
- Attribution for ported work stays in the README Credits section. Renaming
  paths does not change provenance.
