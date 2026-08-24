# Theming — the matugen color pipeline

One wallpaper is the single color source. `mango/scripts/switchwall.sh`
applies the wallpaper and runs matugen, which renders one template per
themed application.

## The rule

**Never edit a generated file.** The next wallpaper switch overwrites it.
Edit the matching file under `matugen/templates/` and run:

```bash
switchwall.sh --noswitch
```

The generated files are listed in `.gitignore` as a safety net. If
`git status` is dirty directly after a theme switch, the
tracked-vs-generated split is broken.

## switchwall.sh

Bound to `SUPER+W`:

```
switchwall.sh                        # rofi thumbnail grid, then set + theme
switchwall.sh [image]                # skip the picker, set this image
switchwall.sh --noswitch             # re-theme, keep the wallpaper
switchwall.sh --mode light|dark
switchwall.sh --type scheme-tonal-spot   # or scheme-expressive, ...
switchwall.sh --color RRGGBB         # theme from a color, no wallpaper
```

The picker (`rofi/wallpaper.sh`) lists the images under `~/Wallpapers` as a
thumbnail grid.

The script reads its intent from `~/.config/mango/theme.json`
(`background.wallpaperPath`, `appearance.palette.{accentColor,type}`) and
writes the first two back on each switch. It applies the wallpaper with
`swaybg` first — mango shows a bare root color otherwise — and then runs
matugen.

**Spanning wallpapers.** When the image aspect matches the total monitor
layout within 10% (for example a 3840x1080 panorama across two 1080p
outputs), the script cuts one tile per output with imagemagick and runs one
`swaybg` group per monitor. Tiles cache under
`~/.local/state/mango/generated/wallpaper/`. Other aspects fill each output
with the whole image. Nothing reruns this on monitor hotplug — press
`SUPER+W` again after you replug.

After matugen, the script switches the theme *names* that matugen cannot
recolor: `adw-gtk3`/`adw-gtk3-dark` (GTK) and `breeze-plus`/
`breeze-plus-dark` (icons) via `gsettings` and `kwriteconfig6`. The
`--notify` flag on `kwriteconfig6` makes running KDE applications repaint
without a restart.

The bar's `darkmode` button toggles light/dark: it flips the gsettings color
scheme and reruns `switchwall.sh --noswitch`.

## Templates

`matugen/config.toml` fans the source color out to every themed app:

| Template | Output | Reload |
|---|---|---|
| `kitty/theme.conf` | `~/.config/kitty/theme.conf` | `pkill -USR1 kitty` |
| `mango/colors.conf` | `~/.config/mango/colors.conf` | mango sources it live |
| `ironbar/style.css` | `~/.config/ironbar/style.css` | ironbar restart (bar start script) |
| `swaylock/config` | `~/.config/swaylock/config` | read fresh on each lock |
| `mako/config` | `~/.config/mako/config` | `makoctl reload` |
| `rofi/colors.rasi` | `~/.config/rofi/colors.rasi` | read fresh on each launch |
| `gtk-3.0/gtk.css`, `gtk-4.0/gtk.css` | `~/.config/gtk-{3,4}.0/gtk.css` | — |
| `wlogout/style.css` | `~/.config/wlogout/style.css` | read on each menu open |
| `kde/kdeglobals` | `~/.config/kdeglobals` | `kwriteconfig6 --notify` |
| `kde/color.txt` | state dir `color.txt` | read by `vscode-set-color.sh` |
| `colors.json` | state dir `colors.json` | read by `keybinds-cheatsheet.py` |
| `wallpaper.txt` | state dir `wallpaper/path.txt` | — |
| `console/palette.conf` | state dir `console-palette.conf` | `console-palette-sync` (boot prompt) |
| `sddm/Colors.qml` | state dir `sddm-colors.qml` | `sddm-theme-sync` (login screen) |

"State dir" is `~/.local/state/mango/generated/`.

`[config.custom_colors]` seeds the ANSI terminal colors from a gruvbox-dark
base with `blend = true`, so the terminal palette shifts toward the
wallpaper's hue instead of staying fixed.

**Template trap:** never put matugen slot syntax inside a comment. Tera
expands templates whole, and one bad slot fails the entire matugen run.

## Lock screen (swaylock)

The full swaylock config is generated from
`matugen/templates/swaylock/config`. It embeds the current wallpaper path,
so the lock screen matches the desktop, and maps the ring and text colors
onto the palette. `SUPER+L` locks; a `pgrep` guard prevents duplicate
instances. `indicator-caps-lock` is on — the ring shows caps-lock state.

## Login screen (SDDM)

The greeter theme lives in `system/sddm/theme/` (one `Main.qml`). It follows
the wallpaper, the palette, and light/dark mode.

```bash
sudo system/sddm/install.sh    # once, and after any change under system/sddm/
```

Preview without an install:

```bash
sed -E 's/"\{\{[^"]*\}\}"/"#808080"/g' matugen/templates/sddm/Colors.qml \
    > system/sddm/theme/Colors.qml
sddm-greeter-qt6 --test-mode --theme system/sddm/theme
```

**How colors reach a root-owned directory.** matugen renders as the user
into the state dir. A post-hook calls `/usr/local/bin/sddm-theme-sync`
through one sudoers rule. The tool is root-owned, takes no arguments, and
installs the render only when it differs from a root-owned reference by
color literals alone. It refuses structural changes, because the greeter
runs this QML before anyone has logged in. After you edit the template, run
the install script again — it refreshes the reference copy.

## Boot prompt (mango-cryptbox)

`mango-cryptbox` draws a themed ASCII frame around systemd's LUKS passphrase
prompt. The palette travels in a small extra early initrd
(`/boot/mango-palette.img`) — the same mechanism Arch uses for microcode —
so a wallpaper switch costs one ~1 KB write, not an `mkinitcpio` run.

```bash
sudo system/cryptbox/install.sh    # once, and after any change under system/cryptbox/
mango-cryptbox --demo              # preview in a terminal
```

The sync tool (`console-palette-sync`) is root-owned, takes no arguments,
and accepts only exactly 16 valid `<index> <rrggbb>` lines. At boot the
frame uses plain ASCII, because the kernel console font has no box-drawing
glyphs, and it always exits 0 — a missing palette means a plain prompt,
never a blocked boot.
