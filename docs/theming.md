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
| `plymouth/colors.conf` | state dir `plymouth-colors.conf` | `plymouth-theme-sync` (boot prompt) |
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

## Boot prompt (plymouth)

The LUKS passphrase prompt is a plymouth theme (`system/plymouth/theme/`)
that looks like the SDDM greeter: the same blurred wallpaper, the same card,
entry and colours. Above the card it shows the clock and, when a TPM secret
is sealed, the boot-attestation code (see below).

```bash
sudo system/plymouth/install.sh    # once, and after any change under system/plymouth/
sudo system/tpm-totp/install.sh    # once; the code on the prompt
```

**How colours reach the initramfs without a rebuild.** The theme script has
no colour and renders no text. Everything the wallpaper decides — the blurred
background, the card, the entry, every glyph of the clock and the code — is a
PNG. matugen renders `plymouth/colors.conf` into the state dir; a post-hook
calls `/usr/local/bin/plymouth-theme-sync` through one sudoers rule. The tool
is root-owned, takes no arguments, validates the nine colour lines and the
wallpaper path, renders the PNGs itself, and packs them into the early initrd
`/boot/mango-plymouth.img`. GRUB loads that image next to the main initramfs;
the kernel unpacks it first, so plymouth finds `themes/mango/dyn/*.png` that
the main image never contains. The same rendered `background.png` is also
copied to `/boot/grub/mango-bg.png`, so `system/grub/install.sh --gfx` shows
the identical blurred wallpaper. A wallpaper switch costs one ~1 MB write.

**Never put a `dyn/` directory into `/usr/share/plymouth/themes/mango/`.**
The stock plymouth hook copies the whole theme directory into the main
initramfs, and files there shadow the early initrd forever. `install.sh`
refuses to run if it exists.

**Early initrds need directory entries.** The kernel unpacks them into an
empty rootfs and creates no parent directories; a cpio holding only files is
dropped without a message. `plymouth-theme-sync` and `console-palette-sync`
both include the parents (the palette one did not before 2026-09-01, and the
"themed" console band was the VT default blue all along).

Preview without touching `/boot`: `plymouth-theme-sync test <dir>` renders
the assets and the cpio from the current state files with no root.

**Handover to SDDM.** plymouth quits with a plain `plymouth quit`; an
sddm.service drop-in (`system/plymouth/sddm-after-plymouth.conf`) makes X
start only after `plymouth --wait` returns. There is a short gap between the
prompt and the greeter — the i915 framebuffer holds whatever was drawn last
before the kernel took over, which with `system/grub/install.sh --gfx` is the
wallpaper GRUB drew, not the ASUS firmware logo. `plymouth quit
--retain-splash` was tried and hung X before it opened `/dev/dri/card1` on
every boot: SDDM has no plymouth handover, unlike GDM, so the seamless
transition is not available here.

To see which DRM device plymouth used and when, add `plymouth.debug` to the
kernel line for one boot (`e` in the GRUB menu) and read
`/var/log/plymouth-debug.log` afterwards.

### Boot attestation code (tpm2-totp)

`system/tpm-totp/` shows a six-digit code in the clock's date slot. It is a
TOTP whose secret sits in the TPM, sealed to PCRs 0, 2, 4 and 7 (firmware,
option ROMs, boot loader, Secure Boot policy). If the firmware or GRUB were
replaced, the TPM refuses to compute it and the prompt says
"TPM mismatch: do not unlock". Compare the code with the phone before typing
the passphrase.

```bash
sudo tpm2-totp -P - -p 0,2,4,7 -l nauthiz generate   # once; password on stdin
sudo tpm2-totp -P - -p 0,2,4,7 reseal                # after a firmware update or enabling Secure Boot
```

`generate` prints an `otpauth://` URI and a QR code. KeePassXC verifies it
without another app: open the entry, *TOTP → Set up TOTP*, paste the secret
from the URI (or use *Custom settings* if the URI says anything but 30 s /
6 digits / SHA1), and the entry shows the current code (*Show TOTP*,
Ctrl+Shift+T). A KeePass client on the phone that syncs the same database
shows the same code, which is the one to compare at boot — the desktop
KeePassXC is not running yet at that point. After login,
`sudo tpm2-totp calculate` against the KeePassXC code is a quick health
check of the TPM state.

The code reaches the prompt as `plymouth update --status=mango:HHMM:CODE`
when the kernel line has `splash`; the theme script parses that grammar.
Without `splash` (the pinned GRUB entry) plymouth shows its text prompt and
`mango-totp` rewrites one line at the top of the console in place — no
`display-message`, which the text view would print as a new line every
second over the passphrase input. Esc on the graphical prompt switches to
the text view; the code is not shown there.

What it does not cover: GRUB here does not measure the kernel or the
initrds (PCR 8/9 are not part of the seal, on purpose — the early initrds
change on every wallpaper switch). A swapped initramfs is only caught by a
UKI booted under Secure Boot (PCR 11), a later project.

## Boot prompt fallback (mango-cryptbox)

`mango-cryptbox` draws a themed band around systemd's *console* LUKS prompt.
It only runs when plymouth is not (`plymouth.enable=0` on the kernel line,
the verbose GRUB entry, or a DRM failure): systemd's console password agent
has `ConditionPathExists=!/run/plymouth/pid`. The palette travels in
`/boot/mango-palette.img`, built by `console-palette-sync` (root-owned, no
arguments, exactly 16 `<index> <rrggbb>` lines).

```bash
sudo system/cryptbox/install.sh    # once, and after any change under system/cryptbox/
mango-cryptbox --demo              # preview in a terminal
```

## GRUB and the rescue entries

`system/grub/install.sh --gfx` hides the menu (F4, Esc or a held Shift
during the 2 s timeout shows it): gfxterm with `/boot/grub/mango-bg.png` as
the background, the same blurred wallpaper plymouth and SDDM show
(`plymouth-theme-sync` keeps it current on every wallpaper switch — run
`~/.config/mango/scripts/switchwall.sh --noswitch` once before the first
`install.sh --gfx` so the file exists). `grub-regen` (the shared, guarded
`grub-mkconfig` wrapper) strips the "Loading Linux ..." echo lines. The
plain console variant (no `--gfx`) is blank on this firmware: the EFI text
console shows no text at all, so the menu cannot be used.

`system/boot-pin/pin-kernel.sh` copies the running kernel and initramfs to
`/boot/pinned/` and adds two entries to `/etc/grub.d/40_custom` that every
`grub-mkconfig` keeps: the pinned kernel (plymouth text prompt, quiet) and a
verbose-console entry for the current kernel (no `quiet`, no `splash`,
plymouth off, mango-cryptbox prompt). The copies must stay out of `/boot`
itself: `10_linux` globs `/boot/vmlinuz-*` and makes the pinned copy the
default entry. Re-run it after a kernel you trust has booted.
