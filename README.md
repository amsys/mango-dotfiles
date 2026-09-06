# mango-dotfiles

Personal Arch Linux desktop configuration for
[mango](https://github.com/DreamMaoMao/mango), a tiling Wayland compositor.

One wallpaper sets the colors of the full desktop.
[matugen](https://github.com/InioX/matugen) makes a Material You palette from
that wallpaper. The palette then applies to the bar, the terminal, the
launcher, the notifications, the lock screen, the GTK and KDE applications,
the login screen, and the LUKS boot prompt.

The status bar is [ironbar](https://github.com/JakeStanger/ironbar). A Rust
daemon in this repo, `mango-bard` (`ironbar/bard/`), collects the system data
and drives the bar.

## Install

```bash
git clone <this repo> ~/src/mango-dotfiles
cd ~/src/mango-dotfiles
./install.sh              # add --dry-run to preview
```

`install.sh` asks before each step. It runs four steps, then prints the
manual steps:

- `install-deps.sh` reports the missing packages. It prints one `pacman` line
  and one AUR line. It installs nothing.
- `install-config.sh` symlinks the configuration into `~/.config`. It also
  builds `mango-bard` into `~/.local/bin`, links the systemd user units, and
  runs the color pipeline one time. It keeps a backup of each file that it
  replaces.
- The root installers in `system/` come next, as a menu. You choose the ones
  this machine needs. The menu marks the five that change what boots or how
  you log in, and it asks a second time before it runs one of those.
- `install-check.sh` verifies the result. It is read-only, and it needs no
  sudo. Run it alone at any time.

Useful flags:

| Flag | Effect |
|---|---|
| `--dry-run` | print every action, change nothing |
| `--yes` / `--no` | answer every question, ask nothing |
| `--system=sddm,rapl` | run those root installers, show no menu |
| `--skip` | leave every file that already exists |
| `--force` | replace an existing file with no backup |
| `--check` | the verify step alone |

Read [docs/install.md](docs/install.md) for the package list, the root-level
installers in `system/`, and the checklist of manual steps.

## Documentation

| Page | Content |
|---|---|
| [install.md](docs/install.md) | installers, packages, `system/` installers, manual checklist |
| [theming.md](docs/theming.md) | the matugen color pipeline, wallpaper switch, login and boot theming |
| [bar.md](docs/bar.md) | ironbar and the `mango-bard` daemon |
| [statusbar-layout.md](docs/statusbar-layout.md) | the normative bar layout specification |
| [keybinds.md](docs/keybinds.md) | keybind rules, rofi modes, screenshots, screen recording |
| [power.md](docs/power.md) | power modes, battery guard, power button, lock before sleep |
| [machine-values.md](docs/machine-values.md) | per-machine values and environment variables |
| [shell.md](docs/shell.md) | fish and git configuration |
| [security.md](docs/security.md) | secrets and the sudo helper design |
| [agents/quirks.md](docs/agents/quirks.md) | rules for coding agents that work on this repo |

## Layout

```
mango/         compositor config, session scripts
ironbar/       bar: mango-bard daemon (Rust), module scripts, service units
rofi/          launcher config, script modes (clipboard, AI chat,
               windows, wallpaper)
matugen/       color pipeline: config, one template per themed application
kitty/         terminal config
hypr/          hypridle config
wlogout/       power menu layout, icons
systemd/       user units (outputs, sleep lock, keep-awake, power key, VNC,
               KDE Connect, arch-update timer)
kdeconnect/    KDE Connect service and desktop files
ollama/        local model files
fish/ git/ fontconfig/ starship.toml
system/        root-installed parts (SDDM theme, boot palette, power modes)
docs/          documentation
memory/        project memory for coding agents
```

Two rules keep the repo clean:

- The installer symlinks one file at a time. It never symlinks a full
  directory. Generated files can then sit next to the symlinks, and they do
  not show in `git status`.
- Do not edit generated output. Edit the template in `matugen/templates/`,
  then run `mango/scripts/switchwall.sh --noswitch`. See
  [docs/theming.md](docs/theming.md).

## Security

This repo tracks no credentials, and it must stay that way. Secrets stay in
the KeePassXC keyring. `mango/scripts/keyring-lookup.sh` reads them at
runtime. See [docs/security.md](docs/security.md).

## Credits

The design of this desktop starts with
[end-4/dots-hyprland](https://github.com/end-4/dots-hyprland) — the
"illogical-impulse" configuration, licensed GPL-3.0. That project is the
source of the visual language and the color architecture here.

Ported from it, with changes: the matugen template set (retargeted from
quickshell QML to this stack), the wallpaper switch script (rewritten for
mango), the bar design, the gruvbox ANSI seed colors, and the keyring and VS
Code color helpers. The path names changed to mango's namespace. The
provenance did not.
