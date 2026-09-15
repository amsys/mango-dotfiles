# Theming — the matugen color pipeline

One wallpaper is the single color source. `mango/scripts/switchwall.sh`
applies the wallpaper and runs matugen. matugen renders one template for
each themed application.

## The rule

**Never edit a generated file.** The next wallpaper switch overwrites it.
Edit the related file in `matugen/templates/`. Then run:

```bash
switchwall.sh --noswitch
```

`.gitignore` lists the generated files as a safety net. If `git status`
shows changes directly after a theme switch, the split between the tracked
files and the generated files is broken.

## switchwall.sh

`SUPER+W` starts the script:

```
switchwall.sh                        # rofi thumbnail grid, then set + theme
switchwall.sh [image]                # skip the picker, set this image
switchwall.sh --noswitch             # re-theme, keep the wallpaper
switchwall.sh --mode light|dark
switchwall.sh --type scheme-tonal-spot   # or scheme-expressive, ...
switchwall.sh --color RRGGBB         # theme from a color, no wallpaper
switchwall.sh --watch-outputs        # started by config.conf, see below
```

The picker (`rofi/wallpaper.sh`) shows the images in `~/Wallpapers` as a
thumbnail grid.

The script reads its intent from `~/.config/mango/theme.json`:
`background.wallpaperPath` and `appearance.palette.{accentColor,type}`. It
writes the first two keys back at each switch. It applies the wallpaper with
`awww` first, because mango shows a bare root color if it does not. Then it
runs matugen. At login, `config.conf` starts `awww-daemon` and shows
`/boot/grub/mango-bg.png`. That is the blurred image which GRUB, the
plymouth prompt and the console framebuffer also show, so the desktop opens
on the boot image. `switchwall.sh --noswitch` then dissolves it into the
sharp wallpaper. `awww` changes the image inside one daemon, thus no layer
surface is dropped and the wallpaper never disappears.

The fade must come from `awww`. mango does not animate a background layer
surface: with `animation_duration_close=1200` the screen showed the root
color 106 ms after the wallpaper client died, and an open fade never ramped
(measured 2026-09-07). A `layerrule` on `layer_name:wallpaper` has no
effect.

**Spanning wallpapers.** The image aspect can agree with the total monitor
layout within 10%, for example a 3840x1080 panorama across two 1080p
outputs. The script then cuts one tile for each output with imagemagick, and
it sends one `awww img -o` for each monitor. The tiles cache in
`~/.local/state/mango/generated/wallpaper/`. A different aspect fills each
output with the whole image.

**Monitor hotplug.** `config.conf` starts `switchwall.sh --watch-outputs` at
login. It reads the `mmsg watch all-monitors` stream, and it applies the
wallpaper again when the set of outputs changes. Both directions need this.
`awww-daemon` runs with `--no-cache`, thus an output which you connect after
login has no image and shows black (`awww query` answers `color: 000000` for
it). An output which stays after you disconnect the other one keeps a tile
with the wrong crop. The watch counts only the outputs which have a size,
because mango announces a monitor before it gives geometry to it. The mode
applies the wallpaper only. It does not run matugen.

After matugen, the script changes the theme *names* that matugen cannot
recolor. These are `adw-gtk3`/`adw-gtk3-dark` for GTK and `breeze-plus`/
`breeze-plus-dark` for the icons. The script uses `gsettings` and
`kwriteconfig6`. The `--notify` flag of `kwriteconfig6` makes the running
KDE applications repaint without a restart.

The `darkmode` button of the bar changes between light and dark. It changes
the gsettings color scheme and runs `switchwall.sh --noswitch` again.

## Templates

`matugen/config.toml` sends the source color to each themed application:

| Template | Output | Reload |
|---|---|---|
| `kitty/theme.conf` | `~/.config/kitty/theme.conf` | `pkill -USR1 kitty` |
| `mango/colors.conf` | `~/.config/mango/colors.conf` | mango reads it live |
| `ironbar/style.css` | `~/.config/ironbar/style.css` | the bar start script restarts ironbar |
| `swaylock/config` | `~/.config/swaylock/config` | read again at each lock |
| `mako/config` | `~/.config/mako/config` | `makoctl reload` |
| `rofi/colors.rasi` | `~/.config/rofi/colors.rasi` | read again at each start |
| `gtk-3.0/gtk.css`, `gtk-4.0/gtk.css` | `~/.config/gtk-{3,4}.0/gtk.css` | — |
| `wlogout/style.css` | `~/.config/wlogout/style.css` | read at each menu open |
| `kde/kdeglobals` | `~/.config/kdeglobals` | `kwriteconfig6 --notify` |
| `kde/color.txt` | state dir `color.txt` | read by `vscode-set-color.sh` |
| `colors.json` | state dir `colors.json` | read by `keybinds-cheatsheet.py` |
| `wallpaper.txt` | state dir `wallpaper/path.txt` | — |
| `plymouth/colors.conf` | state dir `plymouth-colors.conf` | `plymouth-theme-sync` (boot prompt) |
| `sddm/Colors.qml` | state dir `sddm-colors.qml` | `sddm-theme-sync` (login screen) |
| `orca/themes.json.in` | state dir `orca-themes.json` | `orca-set-color.sh`. It patches only while Orca is stopped. The patch applies at login |

"State dir" is `~/.local/state/mango/generated/`.

`[config.custom_colors]` sets the ANSI terminal colors from a gruvbox-dark
base. It uses `blend = true`. The terminal palette thus moves to the hue of
the wallpaper. It does not stay fixed.

**Template trap:** Do not put matugen slot syntax in a comment. Tera expands
the whole template. One bad slot makes the full matugen run fail.

## Lock screen (swaylock)

matugen generates the full swaylock config from
`matugen/templates/swaylock/config`. The config contains the current
wallpaper path, thus the lock screen agrees with the desktop. It also maps
the ring color and the text color onto the palette. `SUPER+L` locks the
screen. A `pgrep` guard prevents a duplicate instance.
`indicator-caps-lock` is on, thus the ring shows the caps-lock state.

## Login screen (SDDM)

The greeter theme is in `system/sddm/theme/` (one `Main.qml`). It follows
the wallpaper, the palette, and the light or dark mode.

```bash
sudo system/sddm/install.sh    # once, and after any change under system/sddm/
```

Preview the theme without an install:

```bash
sed -E 's/"\{\{[^"]*\}\}"/"#808080"/g' matugen/templates/sddm/Colors.qml \
    > system/sddm/theme/Colors.qml
sddm-greeter-qt6 --test-mode --theme system/sddm/theme
```

**How colors reach a root-owned directory.** matugen renders the file as the
user into the state dir. A post-hook calls `/usr/local/bin/sddm-theme-sync`
through one sudoers rule. The tool is root-owned and takes no arguments. It
installs the render only when the render differs from a root-owned reference
in the color literals only. It refuses a structural change, because the
greeter runs this QML before a user logs in. After you edit the template,
run the install script again to refresh the reference copy.

## Boot prompt (plymouth)

The LUKS passphrase prompt is a plymouth theme (`system/plymouth/theme/`).
It looks like the SDDM greeter: the same blurred wallpaper, the same card,
the same entry, and the same colors. Above the card it shows the clock. When
a TPM secret is sealed, it also shows the boot-attestation code (see below).

```bash
sudo system/plymouth/install.sh    # once, and after any change under system/plymouth/
sudo system/tpm-totp/install.sh    # once; the code on the prompt
```

**How colors reach the initramfs without a rebuild.** The theme script has
no color, and it renders no text. Each item that the wallpaper controls is a
PNG: the blurred background, the card, the entry, and each glyph of the
clock and the code. matugen renders `plymouth/colors.conf` into the state
dir. A post-hook calls `/usr/local/bin/plymouth-theme-sync` through one
sudoers rule. The tool is root-owned and takes no arguments. It validates
the nine color lines and the wallpaper path, renders the PNGs itself, and
packs them into the early initrd `/boot/mango-plymouth.img`.

GRUB loads that image next to the main initramfs. The kernel extracts the
early initrd first, thus plymouth finds `themes/mango/dyn/*.png`. The main
image never contains these files. The same rendered `background.png` also
goes to `/boot/grub/mango-bg.png`. `system/grub/install.sh --gfx` thus shows
the identical blurred wallpaper. A wallpaper switch costs one write of
approximately 1 MB.

**Never put a `dyn/` directory into `/usr/share/plymouth/themes/mango/`.**
The stock plymouth hook copies the whole theme directory into the main
initramfs. Files there hide the early initrd permanently. `install.sh` does
not run if this directory exists.

**Early initrds need directory entries.** The kernel extracts them into an
empty rootfs. It creates no parent directory. The kernel drops a cpio that
contains only files, and it shows no message. `plymouth-theme-sync` includes
the parent entries.

To preview without a write to `/boot`, run `plymouth-theme-sync test <dir>`.
The command renders the assets and the cpio from the current state files. It
needs no root.

**Handover to SDDM.** plymouth quits with a plain `plymouth quit`. An
sddm.service drop-in (`system/plymouth/sddm-after-plymouth.conf`) starts X
only after `plymouth --wait` returns. There is a short gap between the
prompt and the greeter. The i915 framebuffer holds the image that was drawn
last before the kernel took control. With `system/grub/install.sh --gfx`,
this image is the wallpaper that GRUB drew, not the ASUS firmware logo.

A test of `plymouth quit --retain-splash` hung X before it opened
`/dev/dri/card1` at each boot. SDDM has no plymouth handover, but GDM has
one. A transition with no gap is thus not possible here.

To find which DRM device plymouth used and when, add `plymouth.debug` to the
kernel line for one boot. Press `e` in the GRUB menu to do this. Then read
`/var/log/plymouth-debug.log`.

The 2026-09-07 debug boot shows the device and the times. plymouthd starts
at 2.39 s. It finds `card0` (simpledrm) first and ignores it, because
plymouth uses a simpledrm device only after the 8 s `DeviceTimeout`. udev
adds `/dev/dri/card1` (i915) at 3.39 s. plymouth then takes that device,
finds connector 508 (eDP-1) already lit, and paints 1920x1080 at 3.44 s.
The screen keeps the GRUB image until that moment. plymouth drops DRM
master at 14.69 s. Two messages in the log are normal: `label-pango.so`
is not in the initramfs, and `fc-match` is not there either. plymouth uses
the freetype label plugin and the bundled `Plymouth.ttf` font instead.

### Reboot watchdog message

At each reboot the kernel prints one line:

```
watchdog: watchdog0: watchdog did not stop!
```

This is not an error. systemd arms the hardware watchdog (`iTCO_wdt`) for
the reboot, because `RebootWatchdogSec` is 10 min by default. It then closes
the device without the magic character, thus the watchdog stays armed
through the switch to `systemd-shutdown`. The kernel prints the line to tell
you that the watchdog still runs. `systemd-shutdown` opens the device again
and prints `Using hardware watchdog /dev/watchdog0`.

The watchdog resets the machine if the reboot stops and does not complete.
This is a laptop with a power button, thus the protection is not necessary.
To remove the message, disable the reboot watchdog:

```bash
sudo mkdir -p /etc/systemd/system.conf.d
printf '[Manager]\nRebootWatchdogSec=off\n' |
	sudo tee /etc/systemd/system.conf.d/10-no-reboot-watchdog.conf
sudo systemctl daemon-reexec   # or the message shows once more
```

PID 1 reads `system.conf` at boot. Without the `daemon-reexec`, the running
PID 1 keeps the old value and prints the message at the next reboot. The
setting is then correct from the boot after that.

### Boot attestation code (tpm2-totp)

`system/tpm-totp/` shows a six-digit code in the date slot of the clock. The
code is a TOTP. Its secret is in the TPM, sealed to PCRs 0, 2, 4 and 7
(firmware, option ROMs, boot loader and kernel image, Secure Boot policy).
If a person replaced the firmware, GRUB or the kernel, the TPM does not
compute the code. The prompt then shows "TPM mismatch: do not unlock".
Compare the code with the phone before you type the passphrase.

```bash
sudo tpm2-totp -P - -p 0,2,4,7 -l nauthiz generate   # once; password on stdin
sudo tpm2-totp -P - -p 0,2,4,7 reseal                # after every kernel upgrade, firmware update or Secure Boot change
```

A `linux` package upgrade moves PCR 4. Under Secure Boot, the firmware loads
the kernel and measures it. The first boot on a new kernel thus always shows
the mismatch icon until you reseal. No component reseals for you.

Use `reseal`. Do not use `generate`. `reseal` keeps the secret. `generate`
makes a new secret. The KeePassXC entry then shows a code that never agrees.

`generate` prints an `otpauth://` URI and a QR code. KeePassXC verifies the
code without another application. Open the entry and select *TOTP → Set up
TOTP*. Paste the secret from the URI. Use *Custom settings* if the URI does
not say 30 s, 6 digits and SHA1. The entry then shows the current code
(*Show TOTP*, Ctrl+Shift+T).

A KeePass client on the phone that syncs the same database shows the same
code. Compare that code at boot, because the desktop KeePassXC does not run
at that time. After login, compare `sudo tpm2-totp calculate` with the
KeePassXC code for a quick health check of the TPM state.

The code reaches the prompt as `plymouth update --status=mango:HHMM:CODE`
when the kernel line has `splash`. The theme script parses that grammar. The
pinned GRUB entry has no `splash`. plymouth then shows its text prompt.
`mango-totp` writes one line again at the top of the console, in place.

`mango-totp` does not use `display-message`. The text view prints such a
message as a new line every second over the passphrase input. Esc on the
graphical prompt changes to the text view. The text view does not show the
code.

The seal does not cover the initrds, but it covers the kernel. GRUB does not
measure the kernel. Under Secure Boot, GRUB loads the kernel with firmware
LoadImage, and the firmware measures that image into PCR 4. The initrds have
no such path. PCR 8 and PCR 9 stay out of the seal on purpose, because the
early initrds change at each wallpaper switch. Only a UKI that boots under
Secure Boot (PCR 11) finds a replaced initramfs, and that is a later
project.

## GRUB and the rescue entries

`system/grub/install.sh --gfx` hides the menu. F4, Esc or a held Shift
during the 2 s timeout shows the menu. The menu uses gfxterm with
`/boot/grub/mango-bg.png` as the background. This is the same blurred
wallpaper that plymouth and SDDM show. `plymouth-theme-sync` keeps the file
current at each wallpaper switch. Run
`~/.config/mango/scripts/switchwall.sh --noswitch` one time before the first
`install.sh --gfx`, so that the file exists.

`grub-regen` is the shared, guarded `grub-mkconfig` wrapper. It removes the
"Loading Linux ..." echo lines. The plain console variant (no `--gfx`) is
blank on this firmware. The EFI text console shows no text, thus you cannot
use the menu.

`--gfx` also pins `GRUB_FONT` to `/boot/grub/fonts/unicode.pf2`, the copy on
the ESP. Without this setting, `/etc/grub.d/00_header` selects
`/usr/share/grub/unicode.pf2` on the encrypted root. `grub.cfg` then mounts
the LUKS volume only to read the font. You type the passphrase one time, at
the plymouth prompt. `cryptomount` thus never prompts, and it fails without
a message.

`loadfont` fails, `gfxterm` never starts, and `background_image` never runs.
GRUB draws nothing. The ASUS logo stays through GRUB and the two handover
gaps, and no component prints an error. `GRUB_TERMINAL_OUTPUT` and
`GRUB_BACKGROUND` are both correct at that time. `grub-regen` does not
install a config that lost the `loadfont /grub/fonts/unicode.pf2` line. This
failure thus cannot come back without a message.

`system/boot-pin/pin-kernel.sh` copies the running kernel and initramfs to
`/boot/pinned/`. It adds two entries to `/etc/grub.d/40_custom`, and each
`grub-mkconfig` keeps them. The first entry is the pinned kernel (plymouth
text prompt, quiet). The second entry is a verbose console entry for the
current kernel (no `quiet`, no `splash`, plymouth off, the console prompt of
systemd). The copies must stay out of `/boot` itself, because `10_linux`
globs `/boot/vmlinuz-*` and makes the pinned copy the default entry. Run the
script again after a kernel that you trust has booted.
