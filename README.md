# mango-dotfiles

Personal Arch Linux desktop configuration for
[mango](https://github.com/DreamMaoMao/mango), a tiling Wayland compositor.

One wallpaper sets the colors for the full desktop.
[matugen](https://github.com/InioX/matugen) generates a Material You palette
and applies it to the bar, the terminal, the launcher, notifications, the
lock screen, GTK and KDE applications, the login screen, and the LUKS boot
prompt.

The status bar is [ironbar](https://github.com/JakeStanger/ironbar). A Rust
daemon in this repo, `mango-bard` (`ironbar/bard/`), collects the system data
and drives the bar.

## Install

```bash
git clone <this repo> ~/src/mango-dotfiles
cd ~/src/mango-dotfiles
./install.sh              # or --dry-run to preview
```

The installer has two parts. `install.sh` runs both, then prints the manual
steps:

- `install-deps.sh` reports missing packages, split into `pacman` and AUR
  lines. It is read-only and installs nothing.
- `install-config.sh` symlinks the configuration into `~/.config`, builds
  `mango-bard`, links the systemd user units, and runs the color pipeline
  once. It backs up each file it replaces.

See [docs/install.md](docs/install.md) for the package list, the root-level
installers under `system/`, and the checklist of manual steps.

## Documentation

| Page | Content |
|---|---|
| [docs/install.md](docs/install.md) | installers, packages, `system/` installers, manual checklist |
| [docs/theming.md](docs/theming.md) | the matugen color pipeline, wallpaper switch, login and boot theming |
| [docs/bar.md](docs/bar.md) | ironbar and the `mango-bard` daemon |
| [docs/keybinds.md](docs/keybinds.md) | keybind rules, rofi modes, screenshots, screen recording |
| [docs/power.md](docs/power.md) | power modes, battery guard, power button, lock before sleep |
| [docs/machine-values.md](docs/machine-values.md) | per-machine values and environment variables |
| [docs/shell.md](docs/shell.md) | fish and git configuration |
| [docs/security.md](docs/security.md) | secrets and the sudo helper design |
| [docs/agents/quirks.md](docs/agents/quirks.md) | rules for coding agents that work on this repo |

## Layout

```
mango/            compositor config + session scripts
ironbar/          bar: bard daemon (Rust), module scripts, service unit
rofi/             launcher config + script modes (clipboard, AI chat, windows, wallpaper)
matugen/          color pipeline: config + one template per themed app
kitty/            terminal config
fish/ git/ fontconfig/ starship.toml
wlogout/          power menu layout + icons
hypr/             hypridle config
systemd/          user units (sleep lock, keep-awake, VNC, KDE Connect)
system/           root-installed parts (SDDM theme, boot palette, power modes, ...)
docs/             documentation
```

Two rules keep the repo clean:

- The installer symlinks files one by one, never whole directories. Generated
  files can then sit next to the symlinks without appearing in `git status`.
- Never edit generated output. Edit the template under `matugen/templates/`
  and run `switchwall.sh --noswitch`. See
  [docs/theming.md](docs/theming.md).

## Security

No credentials are tracked here, and none should ever be. Secrets stay in the
KeePassXC keyring and are read at runtime through
`mango/scripts/keyring-lookup.sh`. See [docs/security.md](docs/security.md).

## Credits

The design of this desktop originates with
[end-4/dots-hyprland](https://github.com/end-4/dots-hyprland) — the
"illogical-impulse" configuration, licensed GPL-3.0. That project is the
source of the visual language and the color architecture here.

Ported from it, with modification: the matugen template set (retargeted from
quickshell QML at this stack), the wallpaper switch script (rewritten for
mango), the bar's visual design, the gruvbox ANSI seed colors, and the
keyring and VS Code color helpers. Path names changed to mango's namespace;
the provenance did not.
