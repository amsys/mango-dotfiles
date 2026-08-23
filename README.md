# mango-dotfiles

Personal desktop config for [mango](https://github.com/DreamMaoMao/mango) (a
scroller/tiling Wayland compositor), themed end-to-end with
[matugen](https://github.com/InioX/matugen) Material You colour generation:
waybar, kitty, rofi, mako, swaylock, GTK, Qt/KDE apps, wlogout, VS Code — all
derived from one wallpaper.

## Install

```bash
git clone <this repo> ~/src/mango-dotfiles
cd ~/src/mango-dotfiles
./install.sh              # or --dry-run to preview
```

The installer is split in two, and `install.sh` just runs both in order:

| Script | Does | Touches your system? |
|---|---|---|
| `install-deps.sh` | reports which packages are missing, split into `pacman` vs AUR install lines | **no** — read-only, never installs |
| `install-config.sh` | per-file symlinks the repo into `~/.config`, aliases the Nextcloud tray icon names into `~/.local/share/icons`, seeds per-machine files, runs the colour pipeline once | yes |
| `install.sh` | both of the above, plus the manual-steps epilogue | yes |

Run either half on its own — `./install-deps.sh` to check packages before
committing to anything, `./install-config.sh` to relink after adding a file.
Both take `--dry-run`.

Symlinks are **per-file, never per-directory** (see
[Layout](#layout--the-tracked-vs-generated-rule)) so matugen's generated output
can sit as a plain file next to them without dirtying this repo. Nothing is
overwritten without a `<file>.bak.<timestamp>` backup first, re-running is a
no-op if everything is already linked, and git is never touched (no init, no
commit) — that stays an explicit, separate step.

`~/.local/share/icons/hicolor/*/status/state-*.png` is the one thing linked
outside `~/.config`, and it points at `/usr/share`, not at this repo. The
Nextcloud client advertises its tray icon as a bare name (`state-ok`,
`state-sync`, …) with no `IconThemePath` and no `IconPixmap` — names that exist
only in Breeze (confirmed still true on 34.0.1daily). Under Adwaita waybar
cannot resolve them and falls back to a generic placeholder, so the installer
aliases them onto the branded icons `nextcloud-client` already ships. hicolor
is the target because every GTK icon theme falls back to it. `state-offline`
and `state-pause` have no branded artwork and land on the plain cloud
alongside `state-ok`.

The UI font is **IBM Plex Sans** (`ttf-ibm-plex`, in the Arch repos —
`install-deps.sh` reports it like any other package, no manual step).

Dependencies `install-deps.sh` checks for. It is report-only — it won't install
for you, since that needs root and several of these are AUR-only. It sorts what's
missing into a `sudo pacman -S` line and a `yay -S` line so you don't get handed
a command that can't work:

| Group | Packages |
|---|---|
| Core | `mangowm-git`ᴬ `waybar kitty rofi-wayland mako` `wlogout`ᴬ `swaylock hypridle matugen swaybg cliphist wl-clipboard` |
| Scripts/keybinds | `grim slurp swappy hyprpicker tesseract tesseract-data-eng brightnessctl playerctl wireplumber networkmanager nm-connection-editor blueman pavucontrol-qt jq libnotify xdg-user-dirs kdialog atop btop dmidecode` |
| Shell/look | `fish starship eza ttf-jetbrains-mono-nerd` `adw-gtk-theme-git`ᴬ `breeze-plus`ᴬ `kde-cli-tools` |
| Optional (feature-gated) | `keepassxc nextcloud-client dolphin` `arch-update`ᴬ |

ᴬ = AUR. `adw-gtk-theme-git` is what provides the `adw-gtk3`/`adw-gtk3-dark`
themes `switchwall.sh` switches between; `breeze-plus` provides the
`breeze-plus`/`breeze-plus-dark` icon themes it pairs with them.

`atop` feeds the CPU tooltip's "recent peaks" list and needs its service running
(see the checklist); `btop` is what clicking the CPU or memory module opens;
`dmidecode` is read once at install time to cache DIMM details for the memory
tooltip. All three degrade quietly — the tooltip drops the section it cannot
fill.

## How the colour pipeline works (matugen)

`mango/scripts/switchwall.sh` is the entry point, bound to `SUPER+W`:

```
switchwall.sh [image]                # pick a new wallpaper interactively
switchwall.sh --noswitch             # re-theme without changing wallpaper
switchwall.sh --mode light|dark
switchwall.sh --type scheme-tonal-spot   # or scheme-expressive, -fidelity, ...
switchwall.sh --color RRGGBB         # theme from a color, no wallpaper
```

It reads intent from `~/.config/mango/theme.json`
(`background.wallpaperPath`, `appearance.palette.{accentColor,type}`,
`appearance.wallpaperTheming.enableAppsAndShell`) and writes the first two
back when you switch,
applies the wallpaper via `swaybg` **first** — mango shows a bare
`rootcolor` otherwise — then runs `matugen`.

`matugen/config.toml` fans one source colour out to every themed app:

| Template | Output | Reload |
|---|---|---|
| `kitty/theme.conf` | `~/.config/kitty/theme.conf` | `pkill -USR1 kitty` |
| `mango/colors.conf` | `~/.config/mango/colors.conf` | (mango reads it live via `source=`) |
| `waybar/style.css` | `~/.config/waybar/style.css` | `pkill -SIGUSR2 waybar` |
| `waybar/claudebar.sh` | `~/.config/waybar/scripts/claudebar.sh` | — |
| `swaylock/config` | `~/.config/swaylock/config` | — (read fresh on each lock) |
| `mako/config` | `~/.config/mako/config` | `makoctl reload` |
| `rofi/colors.rasi` | `~/.config/rofi/colors.rasi` | — (read fresh on each launch) |
| `fuzzel/fuzzel_theme.ini` | `~/.config/fuzzel/fuzzel_theme.ini` | — |
| `gtk-3.0/gtk.css`, `gtk-4.0/gtk.css` | `~/.config/gtk-{3,4}.0/gtk.css` | — |
| `hyprland/{colors,hyprlock-colors}.conf` | `~/.config/hypr/...` | harmless leftover, mango doesn't read Hyprland config |
| `kde/color.txt` | `~/.local/state/mango/generated/color.txt` | read by `vscode-set-color.sh` |
| `kde/kdeglobals` | `~/.config/kdeglobals` | `kwriteconfig6 --notify` |
| `colors.json` | `~/.local/state/mango/generated/colors.json` | read by `keybinds-cheatsheet.py` |
| `wallpaper.txt` | `~/.local/state/mango/generated/wallpaper/path.txt` | — |

`[config.custom_colors]` in `config.toml` seeds the ANSI terminal colours
from a gruvbox-dark base (`term1`–`term6`) with `blend = true`, so they
harmonize toward the wallpaper's hue instead of staying fixed — this replaces
upstream's `generate_colors_material.py` entirely (see [Credits](#credits)).

After matugen, `switchwall.sh` does the part matugen can't: icon theme and
GTK theme are **name** switches, not colour edits
(`breeze-plus-dark`/`breeze-plus`, `adw-gtk3-dark`/`adw-gtk3`) via
`kwriteconfig6`/`gsettings`; `kwriteconfig6 ... --notify` is what makes
already-running KDE apps repaint without a restart.

**Rule: never hand-edit a generated file.** Edit the matching file under
`matugen/templates/`, then run `switchwall.sh --noswitch`.

## Keybinds

`SUPER+/` opens the generated cheat sheet — `mango/scripts/keybinds-cheatsheet.py`
parses `config.conf`, so it is never out of date. The convention behind it:

| Modifier | Owns |
|---|---|
| `ALT` | windows and apps — focus, launch, kill, tag assignment |
| `SUPER` | the desktop — tag navigation, moving windows, session |
| `CTRL` | **nothing.** Left to applications. |

That last row is the rule worth keeping. A compositor bind never reaches the
focused app, so anything on plain `CTRL` is taken away from *every* program at
once: `Ctrl+←/→` used to eat word-jump in every text field, terminal and URL
bar, `Ctrl+Shift+←/→` ate select-by-word, and `Ctrl+1..9` ate tab switching in
browsers, kitty and VS Code. Those live on `SUPER` now. `Ctrl+Print` is the one
exception, and `Print` is not a text-editing key.

Two navigation binds worth knowing, since neither is obvious:

| Key | Does |
|---|---|
| ``SUPER+` `` | `focuslast` — jump back to the previous window |
| ``Alt+` `` | the rofi window switcher: every window on every tag, minimized included |
| `Alt+Tab` | `togglejump` — mango's jump labels, *not* an alt-tab switcher |

Screen recording is a toggle, which is the one thing about it you have to know:

| Key | Does |
|---|---|
| `Alt+Print` | start recording a region; press again to stop |
| `Alt+Shift+Print` | same, whole output, no region selection |

`wf-recorder` has no toggle of its own — it runs in the foreground until
signalled — so `mango/scripts/screenrecord.sh` wraps both ends. It stops with
`SIGINT` specifically: wf-recorder traps it to flush the muxer and write the
moov atom, and killed any other way the `.mp4` exists but will not play. Output
lands in `~/Videos/Recordings/`.

| Key | Does |
|---|---|
| `Alt+S` / `Ctrl+Print` | screenshot the active monitor, save + copy |
| `Alt+Shift+S` | screenshot a selection, save + copy |
| `Print` | screenshot everything, clipboard only |
| `Shift+Print` | screenshot a selection, opens in `swappy` to annotate |

The save/copy pair share `mango/scripts/screenshot.sh` so the destination
directory lives in one place (`MANGO_SCREENSHOT_DIR`, see below) instead of
being duplicated per bind. "Active monitor" comes from `mmsg get
all-monitors`' `active` flag — the compositor's own `selmon`, not a
cursor-position guess.

The window switcher exists because rofi-wayland's built-in `window` mode is
X11/EWMH only. `rofi/window.sh` builds the list from `mmsg get all-clients` and
selects with a single `mmsg dispatch focusid client,<id>`: `focusid` runs
`client_active()`, which views the window's tag, un-minimizes it and focuses it.
Minimized windows are listed on purpose — this is the only way back to a
*specific* one, since `SUPER+Shift+I` restores blindly.

## rofi

`config.rasi` is the static layout (kept in this repo); `colors.rasi` is
matugen output, regenerated on every wallpaper switch — don't edit it.

Modes, wired up in `configuration.modes`:

| Mode | Keybind | What |
|---|---|---|
| `drun` | `Alt+Space` | app launcher |
| `clipboard` | `Alt+V` | `rofi/clipboard.sh --launch` → last 500 `cliphist` entries, inline thumbnails for images, `Alt+D` deletes, `Alt+Shift+D` clears all (two presses) |
| `recursivebrowser` | `Alt+/` | file browser |
| `calc` | `Alt+=` | calculator |
| `ai` | `Alt+I` | `rofi/ai.sh --launch` → multi-turn AI chat with history |
| `window` | ``Alt+` `` | `rofi/window.sh --launch` → every window on every tag, minimized ones included |

`ai.sh` is a chat, not a one-shot question box. The listview *is* the transcript:
each message is wrapped to the window width and every wrapped line is its own
row, so a long answer simply takes more rows (rofi has no variable-height rows).
Follow-ups carry the whole conversation, so context works.

| Key | Does |
|---|---|
| `Enter` | send what you typed — `kb-accept-custom` is rebound to Return, because in a chat typing is the primary action |
| `Ctrl+Enter` | copy the highlighted message in full |
| `Alt+N` | new chat |
| `Alt+D` | clear the current chat |
| `Alt+[` / `Alt+]` | previous / next conversation — not `Alt+←/→`, mango grabs those for `focusdir` |
| `Alt+Shift+D` | wipe all history |
| `Alt+R` | refresh — pull in an answer that has landed |

Conversations live in `~/.local/share/rofi-ai/chats/<epoch>.json`, capped at the
20 newest and 40 messages each. `rofi/ai.rasi` is the chat's own theme (720px,
`@import`s `config.rasi`); the keybindings are passed by `--launch`, which is
why `mango/config.conf` binds the script rather than `rofi -show ai`.

Requests go to OpenRouter (`deepseek/deepseek-r1-distill-llama-70b:free`, or
`$MANGO_AI_MODEL`). The API key comes from the KeePassXC Secret Service via
`mango/scripts/keyring-lookup.sh`; keyring locked, no entry, no key and API
errors are all surfaced in the caption line, and a failed call takes the
question back off the transcript so there is nothing half-sent left behind.
`ai.sh test` self-checks the wrapping, pruning, history navigation and the
pending/reap state machine.

The call runs **detached**. Your question is stored and drawn immediately with a
`···` placeholder, `ai.sh --fetch` is spawned to do the curl, and the answer is
merged in on the next invocation — so the window stays live (scroll, copy,
switch conversations) instead of freezing for the length of the request. rofi's
script protocol has no timer, so **the transcript does not refresh by itself**:
press `Alt+R` once the answer lands. One request is allowed in flight at a time,
and a worker that dies without writing anything is given up on after 120s (curl
itself is capped at 90s).

The lookup is `secret-tool lookup application mango`, so the KeePassXC entry
holding the key needs an **additional attribute** `application` = `mango`
(Entry → Advanced → Additional attributes). Without it, Alt+I reports "no key
found". `keyring-lookup.sh test` self-checks its dbus parsing.

`keyring-lookup.sh` checks the collection's lock state *before* calling
`secret-tool`, not after. A lookup against a locked collection blocks on an
unlock prompt, and `ai.sh`'s detached worker has no terminal to answer one — so
the old order turned "vault locked" into a request that hung forever.

## swaylock

The **entire config is matugen-generated** from
`matugen/templates/swaylock/config` — never hand-edit
`~/.config/swaylock/config`, it gets overwritten on the next wallpaper
switch. It embeds the current wallpaper path directly (`image=...`) so the
lock screen matches the desktop, and maps ring/inside/text colour roles onto
the Material You palette (`ring-ver-color` = verifying, `-wrong-color` = bad
password, `key-hl-color`/`bs-hl-color` = keystroke feedback).

- Bound to `SUPER+L` in `mango/config.conf`, guarded by
  `pgrep -x swaylock || swaylock` so repeated presses don't spawn duplicates.
- Auto-triggered by `hypridle` (see [Checklist](#checklist)) after 5 minutes
  idle, via `loginctl lock-session`.
- `indicator-caps-lock` is on — the ring shows caps-lock state directly.

To restyle: edit `matugen/templates/swaylock/config`, run
`switchwall.sh --noswitch`.

## Machine-specific values

| Value | Where | How to change |
|---|---|---|
| `XDG_DATA_DIRS` | `mango/local.conf` only | not set in `config.conf`: the user flatpak exports dir lives under `$HOME` and mango's `env=` parser does a literal `setenv` with no expansion, so it has to be spelled out absolutely per machine. Without it `kbuildsycoca6` builds an empty app database and Dolphin's "Open With" list is blank |
| `XDG_MENU_PREFIX=plasma-` | `mango/config.conf` | only `plasma-applications.menu` exists in `/etc/xdg/menus` on this machine |
| `monitorrule=name:^eDP-1$,...` | `mango/config.conf` | override in `mango/local.conf` — find your output name with `wlr-randr` or `mmsg get outputs` |
| `DEV` (wifi device, default `wlo1`) | `waybar/scripts/{wifi-menu,net,net-watch}.sh` | set `MANGO_WIFI_DEV` env var |
| `DEV` (ethernet device, default `eno2`) | `waybar/scripts/{net,eth-toggle}.sh` | set `MANGO_ETH_DEV` env var |
| KeePassXC database path | `mango/scripts/keepassxc-autounlock.sh` | set `env=MANGO_KEEPASS_DB,/path/to/db.kdbx` in `mango/local.conf`. There is no default — the script exits and you unlock by hand if it is unset. It greps `local.conf` rather than reading the environment because mango applies `env=` only *after* forking `exec-once` children, and this script is one |
| `/run/keepassxc-unlock/$USER` stash file | same script | populated by `/usr/local/bin/keepassxc-stash-pw`, **not in this repo** — see checklist |
| KeePassXC attribute for the OpenRouter key | `mango/scripts/keyring-lookup.sh` | defaults to `mango`; pass a different one as `$1` |
| World clock zones in the clock tooltip | `waybar/scripts/clock.sh` | set `MANGO_WORLD_TZ` to a comma-separated zone list (default `America/New_York,Europe/Prague,Asia/Bangkok,Indian/Mauritius`); the local zone is skipped in the list because the Local row already shows it |
| Pomodoro durations | `waybar/scripts/clock.sh` | set `MANGO_POMODORO` to `work,short,long,cycles` in minutes (default `25,5,15,4`) |
| Calendar app the date click opens | `waybar/scripts/clock.sh` | set `MANGO_CALENDAR_CMD` (default `thunderbird -calendar`) and `MANGO_CALENDAR_APPID` (default `org.mozilla.Thunderbird`) — the appid is what `mmsg get all-clients` is searched for, so both have to change together |
| DIMM details in the memory tooltip | `~/.cache/mango-meminfo` | written once by `install-config.sh` from `sudo dmidecode -t memory`; delete it and re-run to refresh, or set `MANGO_DMI_CACHE` |
| atop log directory | `waybar/scripts/cpu.sh` | set `MANGO_ATOP_DIR` (default `/var/log/atop`) |
| AI chat storage / model | `rofi/ai.sh` | set `MANGO_AI_DIR` (default `~/.local/share/rofi-ai`) or `MANGO_AI_MODEL` |
| Low-battery thresholds | `mango/scripts/battery-guard.sh` | set `MANGO_BATTERY` to `warn,critical,action,floor` in percent (default `20,10,5,3`); `MANGO_BAT_DIR`/`MANGO_AC_DIR` are shared with `battery.sh` |
| Charger chime | `mango/scripts/ac-watch.sh` | `MANGO_AC_DIR`/`MANGO_BAT_DIR`, the same pair `battery.sh` and `battery-guard.sh` use — all three glob `A[CD]*`/`BAT*` if unset |
| Power menu size | `mango/scripts/powermenu.sh` | set `MANGO_POWERMENU_SIZE` to `WIDTHxHEIGHT` in pixels (default `760x430`); the margins that centre it are derived from the active monitor, not hardcoded |
| What counts as "busy" | `mango/scripts/busy.sh` | set `MANGO_DOWNLOAD_DIR` (default `~/Downloads`), `MANGO_DOWNLOAD_FRESH_MIN` (default `5`, so an abandoned `.part` stops blocking suspend), `MANGO_PACMAN_LCK` |
| First-hover tooltip delay | `waybar/fast-tooltips.c` | set `MANGO_TOOLTIP_DELAY_MS` (default `80`); only takes effect if the LD_PRELOAD shim built, see "The system indicators" below. Applies to waybar only — ironbar's hover popups (IRONBAR.md) carry no GTK tooltip at all and use their own `HOVER_DELAY` constant in `mango-bard`'s `main.rs` (80ms, not env-configurable) |
| Screenshot save directory | `mango/scripts/screenshot.sh` | set `env=MANGO_SCREENSHOT_DIR,/path` in `mango/local.conf` (default `~/Pictures/Screenshots`); a leading `~/` is expanded, mango's `env=` does not do it itself |

`mango/config.conf` ends with `source=./local.conf`, installed once from
`mango/local.conf.example` and never overwritten — put per-machine
overrides there.

## Layout & the tracked-vs-generated rule

```
mango/            compositor config + scripts
waybar/           bar config + scripts
kitty/            terminal config
rofi/             launcher config + script-mode backends
matugen/          the color pipeline: config.toml + every template
wlogout/          power menu: layout + icons (style.css is matugen output)
fish/             shell config (excl. secrets, fish-managed state)
git/              global git config + global gitignore
fontconfig/       font rendering tweaks
starship.toml
install.sh        runs both halves below, then prints manual steps
install-deps.sh   package check (read-only)
install-config.sh symlinks + per-machine files + colour pipeline
```

`fish/config.fish` defines abbreviations rather than aliases (`pi`, `pu`, `pss`,
`ys`/`yi`/`yr`, `gs`/`gc`/`gp`/`gd`/`gl`/`gsw`/`gcl`, `scu`/`scs`/`jcu`, `psg`,
`duh`, `tf`, `cr`). An abbreviation expands in place, so the real command is
visible before you press enter, is editable (add a `-y`, change a flag), and
lands in history as what actually ran — none of which an alias gives you, and it
cannot shadow a binary either. `abbr --show` lists them all.

It also wires up four tools that were installed on this machine and doing
nothing, which is worth stating because none of them announce themselves:

| Line in `config.fish` | Gives you |
|---|---|
| `zoxide init fish \| source` | `z partial-name` jumps to the dir you use most; `zi` picks interactively |
| `fzf_key_bindings` | `Ctrl+R` fuzzy history, `Ctrl+T` insert a path, `Alt+C` cd into a subdir |
| `function y` | yazi, but quitting leaves the shell in the directory you ended up in |
| `MANPAGER` | man pages through `bat`, syntax-highlighted |

`fzf` in particular ships `fzf_key_bindings` as a vendor function and never
calls it, so a stock install has no bindings at all — the package looks present
and does nothing. The call has to live in `config.fish` rather than `conf.d/`,
because `conf.d/` loads first and fish's own binding setup would overwrite it.

### git

`git/config` is the global git config, moved in here from `~/.gitconfig` so it
stops being the one piece of shell state living outside the repo. It sets
`git-delta` as the pager.

The migration has a trap in it: git reads `$XDG_CONFIG_HOME/git/config` **first**
and `~/.gitconfig` **second**, and the later file wins. So the tracked file does
nothing at all until `~/.gitconfig` is deleted — edits appear to apply and
silently don't. `install.sh` prints this as a manual step.

No delta theme is set on purpose. Matugen owns colour here, and a pinned delta
theme would drift from the palette at the next wallpaper switch; delta falls
back to `$BAT_THEME`, which is the hook if it ever needs theming.

### The network indicators

Three waybar modules and one watcher, all in `waybar/scripts/`:

| Module | Script | Notes |
|---|---|---|
| `custom/netwatch` | `net-watch.sh` | Continuous module (no `interval`). Blocks on `nmcli monitor`, streams one spinner frame per line, and `pkill`s **RTMIN+10 and RTMIN+12 together** on every NM event — that pairing is what keeps the security lock and the signal readout from disagreeing for up to a poll interval. `restart-interval: 5` respawns it if NetworkManager restarts. |
| `custom/wifi` | `net.sh --wifi` | Emits **empty text** (waybar hides the module) whenever the device is mid-transition, or while `wifi-menu.sh` is blocked on a rescan, so the spinner *replaces* the signal arc rather than appearing beside it. |
| `custom/eth` | `net.sh --eth` | Shares signal 12 with the Wi-Fi module. |
| `custom/netsec` | `net.sh --sec` | Signal 10. |

The scan spinner is the one case NM cannot report — a rescan changes no device
state — so `wifi-menu.sh` signals `net-watch.sh` by hand (`USR1`/`USR2`) and
drops its pid in `$XDG_RUNTIME_DIR/wifi-scan`, which is how `net.sh` knows to
hide the Wi-Fi module and how it recovers if the menu is killed outright.

Both scripts self-check: `net.sh --selftest` and `net-watch.sh test`.

`volume.sh --watch` and `workspace.sh --watch` share that watcher shape through
`waybar/scripts/watch.sh` (sourced, bash-only). The one thing it exists to get
right is `coproc` rather than `cmd | while read`: in a pipeline the stream
producer is a sibling process, so terminating the module kills the script and
leaves `mmsg watch` / `pactl subscribe` running forever — one more leaked every
waybar restart. `net-watch.sh` was already built this way, which is where the
pattern comes from.

> `mango/scripts/mango-window.sh` still uses the pipeline form and does leak an
> `mmsg watch focusing-client` per waybar restart (15 of them on this machine at
> the time of writing). Same one-line fix via `watch.sh`; not done here.

### One bar per monitor

Every mango monitor has its own independent set of nine tags — two screens can
sit on tag 9 at once with entirely different windows on each. waybar cannot tell
a custom module which output its bar is on (no `--output` flag, no environment,
no placeholder), so the monitor name has to reach `workspace.sh` through its
`exec` line, which means one bar object per output and therefore a generated
config.

`waybar/scripts/bars.sh` is what `exec-once` launches instead of bare `waybar`.
It reads `mmsg get all-monitors` and writes an array of bar objects to
`$XDG_RUNTIME_DIR/waybar-bars.json`, each one overriding **nothing but the nine
`exec` strings** and `include`-ing `config.jsonc` underneath — waybar merges
nested objects, so `signal`, `escape`, `return-type` and the click bindings all
still come from the tracked file, which stays the only place the pills are
defined. Run `waybar` bare and it still works; you just get one monitor's tags
on every screen, which is the bug this replaces.

Two details worth keeping:

- **The last array element is a catch-all** — `output: ["!eDP-1", "!DP-1", "*"]`
  — so a monitor plugged in after login gets a bar *immediately*, before
  anything regenerates. Pinning bars to names without it would mean a new output
  comes up with no bar at all, which is worse than the bug.
- **Hotplug restarts waybar; it does not reload it.** `bars.sh` follows `mmsg
  watch all-monitors`, and when the set of names actually changes it regenerates
  and restarts. `SIGUSR2` is not enough: waybar's reload re-reads the config but
  does **not** rebuild the bar list from it, so a bar for a newly connected
  output never appears — and in testing the bars it already had went away too.
  The restart is safe for the tray: `config.conf`'s `arch-update` note is about
  the applet's *startup*, when no `StatusNotifierHost` is listening yet; a
  running one reconnects, verified across several restarts.

Clicks need no monitor of their own: mango sets `selmon` from the cursor on
every button press and on motion under `sloppyfocus`, so `view,N,0` and the
scroll binds already act on the screen whose bar you are touching.

`bars.sh test` checks the generator against canned IPC output.

### The system indicators

`cpu`, `memory`, `battery`, the clock, volume and the workspace pills are
scripts too, for the same reason
the network ones are: a built-in module's tooltip is a fixed format string, so
it cannot draw meters, name the process responsible, or show battery wear. They
share the tooltip vocabulary — palette, `██░░` meters, section headers, the JSON
emitter — via `waybar/scripts/tooltip.sh`, which is sourced, never executed.

`rule` takes an optional glyph count, and that is what pins a tooltip's width:
one repeated glyph has one advance, so N dashes is a deterministic pixel width
even in a proportional font. Make the rule the widest line and the tooltip stops
resizing with its content *and* gets equal left and right margins — `row`/`dim`
indent three spaces on the left and nothing on the right, so a row that happens
to be widest sits flush against the right edge. `cpu.sh` and `memory.sh` pass
`rule 44` for that reason; the rest stay variable-width. There is no "Updated
HH:MM" footer: on a module that refreshes every 3-15s it said nothing. claudebar
keeps its own, where a 300s interval makes it real.

GTK3's first-hover tooltip delay is a **compile-time constant** —
`gtk_tooltip_start_delay()` in `gtktooltip.c` hardcodes 500ms (`HOVER_TIMEOUT`)
and the `gtk-tooltip-timeout` GtkSettings property has been dead code since
3.10, so no settings.ini, gsettings or CSS knob reaches it. `waybar/fast-tooltips.c`
works around it with an LD_PRELOAD shim that intercepts the exported
`gdk_threads_add_timeout_full()` symbol from `libgdk-3.so.0` and shortens only
the `(priority 0, 500ms)` case, leaving the 60ms browse-mode timer (and
everything else) untouched. `install-config.sh` compiles it into
`~/.local/lib/mango/fast-tooltips.so`, and `waybar/scripts/bars.sh` adds it to
every waybar launch via `LD_PRELOAD`, falling back to unmodified GTK behaviour
if the `.so` hasn't been built yet. The `.c` source is tracked; the compiled
`.so` is machine-local build output, same split as the DIMM cache mentioned
above. `MANGO_TOOLTIP_DELAY_MS` overrides the 80ms default.

The left pill carries `cpu`, `memory`, `battery`, `claudebar` and `mpris`; the
right one the clock, the util buttons and keep-awake. Battery and claudebar sit
on the left because the left pill has the room and the right one was crowded.

| Module | Script | Notes |
|---|---|---|
| `custom/cpu` | `cpu.sh` | Per-core sparkline, load, package temperature, top processes, anything wedged in `D`/`Z` state for 20s+ (kernel threads excluded — i915's flip worker sits in `D` permanently), and **recent peaks from atop**. `atopsar` takes ~1.3s on a day's log, which stalled waybar's main loop, so it is refreshed into `$XDG_RUNTIME_DIR/waybar-cpu-atop` in the background every 5 minutes and the tooltip renders from the cache. Click opens `btop`. |
| `custom/memory` | `memory.sh` | RAM/cache/dirty, **swap folded in** (it used to be its own pill and read a permanent 0%), minor/major page-fault rates, top processes by RSS, and DIMM details from the install-time `dmidecode` cache. Click opens `btop`. |
| `custom/battery` | `battery.sh` | Charge and **health against design capacity** as meters, cycle count, draw in watts, time to empty/full. Handles both `charge_*` (µAh) and `energy_*` (µWh) batteries. |
| `custom/clock` | `clock.sh` | Replaces the built-in `clock`, which can scroll through timezones but cannot show several at once. Tooltip is the local time large, then the world zones **three to a row** with the city and the offset under each, plus the pomodoro. The enlarged time row is padded in its own cells (`T_W`/`T_GAP` against `COL_W`/`COL_GAP`) so the columns still line up — the self-check asserts the two widths match. Left-click starts/pauses a pomodoro, middle-click mutes it for 30 min (also `SUPER+SHIFT+M`), right-click resets; signal 14 refreshes it immediately after a click. |
| `custom/date` | `clock.sh --date` | Replaces `clock#date`, and is a separate module because waybar hangs the tooltip and the clicks off the module. Tooltip is the current month from `cal -m` drawn as a **bordered table** with today highlighted; the cells are read off `cal`'s fixed 3-character columns rather than matching the number, which would also hit the 2 inside 12. Borders are `C_EMPTY`, deliberately darker than both the day numbers and the weekday header. The table is 36 cells wide, flush left and carries no `rule()` — it is the widest line, so *it* pins the tooltip width and the CSS `tooltip label` padding is the only margin; the self-check asserts every line is that same width. Click focuses the calendar app (`clock.sh --calendar` → `mmsg dispatch focusid`, which switches tag *and* monitor and un-minimizes) and remotes `-calendar` into it rather than starting a second window. |
| `custom/volume` | `volume.sh` | Replaces the built-in `pulseaudio` module — its tooltip can only print waybar's own placeholders, so the active port, the card profile, the mic and the per-app streams had nowhere to go. Left-click opens `pavucontrol-qt`, right-click its Configuration tab, scroll adjusts via `wpctl`. Signal 21, raised by `custom/volwatch` (`volume.sh --watch`) blocking on `pactl subscribe` — never a poll. |
| `custom/ws#1`..`#9` | `workspace.sh N [monitor]` | Nine per-tag pills replacing `ext/workspaces`, which has no `tooltip` option at all. Each bar passes its own output name, so the pills filter `all-tags` and `all-clients` by `.monitor` and every screen shows *its* tags — see "One bar per monitor" above for where that name comes from. Hover lists that tag's windows (`*` focused, `!` urgent, `_` minimized). Click dispatches `mmsg dispatch view,N,0`, which lands on the right screen because the press itself moved `selmon` there. Signal 20, raised by `custom/wswatch` (`workspace.sh --watch`) off `mmsg watch all-tags` — one stream covers every monitor. The `#N` suffix is what keeps the CSS to one rule set: waybar names all nine widgets `custom-ws` and turns the suffix into a style class. |
| `custom/power` | `mango/scripts/powermenu.sh` | Far right of the bar. Opens the same power menu a tap on the power key does. |

Swap on this machine is a cold 4 GiB **swapfile** at priority −1, not zram — the
memory tooltip labels whatever `/proc/swaps` actually reports, so it stays honest
if that changes.

The pomodoro has no daemon: waybar's 1s poll of `clock.sh` *is* the tick, and
the state is four fields in `$XDG_RUNTIME_DIR/mango-pomodoro`. Phase changes
beep through `paplay` and fire a **critical** mako notification (sticks until
dismissed) that replaces its predecessor rather than stacking. Kill waybar
mid-pomodoro and the phase change fires late, when it comes back.

Three guardrails sit on top, all in the same file and all env-overridable:

- **Auto-start on unlock.** `hypr/hypridle.conf`'s `before_sleep_cmd` runs
  swaylock; `clock.sh` notices it was locked (`$XDG_RUNTIME_DIR/mango-pomodoro-lock`)
  and, if no pomodoro is already running, starts a fresh focus block the
  first poll after swaylock exits.
- **Mute for calls.** Middle-click the clock (or `SUPER+SHIFT+M`) silences
  sound and notifications for `MANGO_POMODORO_MUTE` minutes (default 30);
  the timer itself keeps running. A dim bell-off glyph shows in the bar
  while muted.
- **Break warning.** A `hypridle` listener writes `$XDG_RUNTIME_DIR/mango-idle`
  after 30s of no input and removes it on the next keypress. If a break is
  still running 45s in and that flag is absent, `clock.sh` fires a sticky
  "Still working" notification and repeats it every 60s; after 5 minutes of
  being ignored it adds a soft sound. Going idle at any point clears the
  warning clock, so a real break never escalates.

Each self-checks: `cpu.sh test`, `memory.sh test`, `battery.sh test`,
`clock.sh test`, and `sh tooltip.sh tooltip-selftest` for the shared meters.

Everything lives under the compositor's own namespace — `mango/scripts/` holds
the whole script set (wallpaper pipeline, keyring lookup, KeePassXC autounlock,
cheatsheet, VS Code colour sync) and generated state goes to
`~/.local/state/mango/generated/`. Nothing is filed under the name of the
project this was ported from; see [Credits](#credits) for what that was.

Most of what's *visible* in `~/.config` is matugen **output**, not source —
`waybar/style.css`, `kitty/theme.conf`, `swaylock/config`, `mako/config`,
`rofi/colors.rasi`, `mango/colors.conf`, both `gtk.css` files, `kdeglobals`,
and `waybar/scripts/claudebar.sh` are all rewritten on every wallpaper
switch. This repo tracks the **templates** in `matugen/templates/` instead
and materializes the rest via `install.sh` / `switchwall.sh`. They're also
listed in `.gitignore` as a safety net. If `git status` is ever dirty right
after a theme switch, something in this split broke.

Also excluded: `mango/keybinds.html` (generated from `config.conf` by
`keybinds-cheatsheet.py`), `mango/theme.json` (mutated at runtime by
`switchwall.sh`, shipped as `theme.json.example`), `mango/local.conf`
(per-machine, shipped as `local.conf.example`), `git/config.local`
(per-machine identity, shipped as `config.local.example`), and
`fish/conf.d/claude.fish` (machine-local API keys — see below).

## The power button

Three tiers, one of which is not ours:

| | |
|---|---|
| tap (< 1s) | `powermenu.sh` — the session menu, same as the bar's far-right button |
| hold 1-3.5s, then release | `systemctl suspend` |
| hold ~4s | the firmware cuts power. ACPI power-button override, fixed in hardware |

Shutdown is a button in the menu, deliberately not a hold tier: this machine's
override sits at ~4s, so anything scheduled near it would race a power cut with
a clean shutdown and lose.

### The menu

`mango/scripts/powermenu.sh` wraps `wlogout` — nothing invokes `wlogout`
directly any more. It does three things the bare command could not:

- **Warns first.** It runs `busy.sh` and, if a package transaction or an
  unfinished download is in flight, raises a critical notification naming it
  before putting a Shutdown button under the cursor. It informs, it does not
  block — you may well be shutting down *because* something is stuck.
- **Sizes the menu.** wlogout sets `hexpand`/`vexpand` on every button, so CSS
  cannot make the menu smaller: the buttons always fill their grid cell and the
  grid fills whatever its margins leave. The only lever is the `-L/-R/-T/-B`
  margins, which are **pixels**. The script asks the compositor for the active
  monitor (`mmsg get all-monitors`) and centres a fixed-size panel on it, so a
  different display does not need a different number.
- **Keeps the flags in one place**, now that three call sites want them.

The stylesheet is **matugen-generated** like the bar's —
`matugen/templates/wlogout/style.css`, sharing waybar's palette vocabulary, so
the two agree. Never edit `~/.config/wlogout/style.css`.

It paints the rounded panel on the `grid` node, the only widget sitting exactly
behind the buttons (`window > widget > grid > button`). Its `padding` is
load-bearing: GTK3 does not clip children to a parent's `border-radius`, so
without it the square buttons paint over the corners.

The **icons are images, not text** (`wlogout/icons/`, one SVG per button).
wlogout builds buttons with `gtk_button_new_with_label` and never enables
markup — Pango `<span>` renders literally — so the button's own text can only
ever be one size. Making the glyph a `background-image` is the only way to
scale and centre it independently of the label. Each SVG is a `<text>` element
carrying the same Material Symbols Rounded ligature waybar uses for its own
icons, rendered by librsvg via pango, so both draw from one icon vocabulary.

Two dead ends already tried, so they don't get tried again: `-gtk-recolor()`
parses without complaint but silently draws **nothing** on a plain SVG (it
wants GNOME's symbolic format), and the freedesktop `system-*-symbolic` icons
are scattered across three different installed themes and don't visually match.
So the SVG fill is baked in, and the panel stays dark enough to suit it in any
palette.

One parser trap: a literal `/*` anywhere inside a CSS block comment — an
`icons/*.svg` glob, say — desyncs GTK's parser and silently drops **every rule
after it**, which shows up as missing icons rather than as an error.

## Low battery

`mango/scripts/battery-guard.sh`, an `exec-once` that polls every 30s:

| | |
|---|---|
| 20% | notification, soft chime |
| 10% | **persistent** critical notification + alarm, repeating every 5 min |
| 5% | 60s countdown, then `systemctl suspend` |
| 3% | suspends even if something is mid-flight |
| 2% | UPower's own `PercentageAction` — should now be unreachable while awake |

Plugging in at any point cancels a running countdown and re-arms every tier.
Tiers fire once and only re-arm two percent above where they fired, so a
battery hovering on a threshold does not chatter.

`waybar/scripts/battery.sh` already turned the bar glyph red at 20%, but a
recoloured glyph is not a failsafe — it is silent, and invisible if the bar is
covered. Below it there was nothing at all until UPower's `PercentageAction=2.0`
in `/etc/UPower/UPower.conf`, which with `CriticalPowerAction=Auto` and no
working hibernate resolves to an **unclean power off**. Acting at 5% is what
keeps that from ever being reached.

The visual half of the failsafe is free: mako's `[urgency=critical]` block sets
`default-timeout=0`, so `notify-send -u critical` sticks on screen until
dismissed. Note mako also sets `ignore-timeout=1` globally, so `-t` does
nothing — urgency is the only lever.

At 5% the guard consults `mango/scripts/busy.sh` and defers while a pacman
transaction (`/var/lib/pacman/db.lck`) or a fresh partial download
(`~/Downloads/*.part`, `*.crdownload`) is in progress: waking to a half-applied
transaction is worse than spending another percent. At 3% there is no percent
left to spend and it suspends regardless.

`mango/scripts/powerkey.py` reads `/dev/input/event*` for the node named `Power
Button` and times press to release. It is a watcher rather than a keybind
because mango has no release binds — a bind cannot measure a hold at all — and
rather than logind because logind offers one short-press action plus a
long-press whose threshold is a hardcoded 5s, past the firmware override, so it
would never fire. No privileges needed: the user is in the `input` group.

logind has to stand down for any of this to work, which is the
`/etc/systemd/logind.conf.d/10-power.conf` entry on the checklist below. It is
not tracked here because it lives outside `$HOME`. Until it is in place logind
suspends the moment the key goes down and the tap tier is unreachable.

`powerkey.py test` asserts the tier thresholds without needing the device.

### The charger chime

`mango/scripts/ac-watch.sh`, a second `exec-once`: a low-urgency notification
and a light sound each time the adapter goes in or comes out.

| Edge | | Sound |
|---|---|---|
| plugged in | `Charger connected` · `73% · charging` | `power-plug.oga` |
| unplugged | `On battery` · `73% · discharging` | `power-unplug.oga` |

The sound is skipped when the default sink is muted — `wpctl get-volume` appends
`[MUTED]`, which is the whole check. `battery-guard.sh`'s own `beep()`
deliberately does **not** do this: that one is a failsafe alarm, not a courtesy.

It is a separate watcher rather than six lines inside `battery-guard.sh`, which
already polls power state and already resolved `$AC` without ever using it. The
reason is latency, not tidiness: the point of a plug confirmation is catching a
dead brick, a half-seated jack or a port that has stopped negotiating charge,
and the guard's `POLL=30` would answer up to 30s late — while dropping that poll
would multiply the wakeups of a failsafe for a cosmetic feature.

So it blocks on `udevadm monitor --udev --subsystem-match=power_supply`, which
needs no root and no udev rule. It reads the event **count, not the content**:
every wakeup just re-reads `$AC/online` and acts only on a `0`↔`1` edge. That
absorbs udevadm's own two-line startup banner and the `change` events `BAT0`
fires on each capacity tick — both match the same filter — without parsing
anything. Wording comes from `online` rather than `$BAT/status`, because ACPI
lags the transition and can still report `Discharging` a beat after the plug
lands.

Two details that are load-bearing rather than decorative:

- `coproc`, not `udevadm … | while read` — the leak `waybar/scripts/watch.sh`
  exists to avoid, and that `mango-window.sh` still demonstrates.
- an outer retry around the whole thing. `read` hits EOF when `systemd-udevd`
  restarts, which any routine system update does, and an `exec-once` has no
  supervisor behind it the way waybar's `restart-interval` covers its `--watch`
  modules. Without it the chime would die silently and stay dead until the next
  login. The previous state is deliberately *not* re-read across a restart, so a
  cable that moved while the source was down is announced late rather than never.

No time-to-empty in the body: `power_now`/`current_now` read zero for the first
seconds after a transition, which is exactly when this fires, so the estimate
would usually be blank. The battery tooltip in the bar is one hover away and
already computes it properly.

`ac-watch.sh test` asserts the wording and the edge detector without a device.

## Login screen (`mango-sddm`)

The greeter is written from scratch and lives in `system/sddm/theme/` — one
`Main.qml`, a `metadata.desktop`, a `theme.conf`. It tracks matugen like every
other surface: wallpaper, palette, and light/dark all follow a wallpaper switch,
because `switchwall.sh` already passes `--mode` and the rendered `Colors.qml`
*is* the current mode's palette.

```
sudo system/sddm/install.sh        # once, and after any change under system/sddm/
```

Everything under `system/` is installed by that script, never symlinked —
`install-config.sh` only links the `~/.config` directories, so `system/` is
deliberately outside its reach.

Preview it without installing anything:

```bash
sed -E 's/"\{\{[^"]*\}\}"/"#808080"/g' matugen/templates/sddm/Colors.qml \
    > system/sddm/theme/Colors.qml          # scratch render, gitignored
sddm-greeter-qt6 --test-mode --theme system/sddm/theme
```

**How the colours get to a root-owned directory.** matugen renders as you, into
`~/.local/state/mango/generated/sddm-colors.qml`. A post_hook then calls
`/usr/local/bin/sddm-theme-sync` through one sudoers rule. That tool is
root-owned, not user-writable, and takes no arguments — and it installs the
render only if it differs from a root-owned copy of the template by *colour
literals alone*, refusing anything structural. The file in question is QML
executed by the greeter as the `sddm` user before anyone has authenticated, so
"only colours can change" is the property worth having.

This is deliberately not the upstream ii-sddm-theme recipe, which points a
`NOPASSWD` rule at a script inside `$HOME` — that is `NOPASSWD: ALL` in
practice, since you can rewrite the script.

Two things to know when editing:

- Never put matugen slot syntax inside a comment in `matugen/templates/sddm/Colors.qml`.
  Tera expands templates whole; a malformed slot in a comment fails the entire
  matugen run, not just this file.
- After editing that template, re-run `sudo system/sddm/install.sh`. The
  validation reference is a copy, so until you do, the sync refuses to install
  and says so.

Not tracked, because they are generated: `Colors.qml` and `background.*` inside
the installed theme. Both are rewritten on every wallpaper switch.

## Battery power attribution (`system/rapl/`)

`waybar/scripts/battery.sh`'s tooltip can only show total draw without help:
the two sources that know *where* the watts go — RAPL and powertop — are both
root-gated by default.

```
sudo system/rapl/install.sh
```

Like `system/sddm/`, this is installed by a root script, never symlinked.
It does two things:

- A udev rule (`/etc/udev/rules.d/mango-rapl.rules`) group-reads
  `/sys/class/powercap/intel-rapl*/energy_uj` for `wheel`. RAPL is root-only
  by default because of the PLATYPUS side-channel (CVE-2020-8694 — a local
  attacker can infer AES keys from package energy readings); on a single-user
  laptop that risk doesn't apply, and RAPL is the only root-free source of
  per-domain power. A tmpfiles.d `z` line looks like the more obvious tool for
  this, but `intel_rapl_common`/`intel_rapl_msr` are loadable modules on this
  kernel (`CONFIG_INTEL_RAPL=m`), and `systemd-tmpfiles-setup.service` can run
  before they've created the sysfs nodes — a udev `ACTION=="add"` rule fires
  exactly when the node appears instead, every boot.
- `/etc/sudoers.d/mango-powertop`: `%wheel ALL=(root) NOPASSWD: /usr/bin/powertop ""`.
  The trailing `""` forbids arguments, so this grants exactly "run the
  interactive powertop TUI", not root — same reasoning as
  `sudoers.d-sddm-theme-sync`. `custom/battery`'s `on-click` runs it in a
  `mango-monitor`-class kitty window.

powertop's Overview tab ranks devices by estimated power, which needs
`/var/cache/powertop/saved_parameters.powertop`. Until that file exists it has
no power model to rank by, so it falls back to raw activity — software
netdevs (docker veths) outrank real hardware, and the summary line reads
`-nan wakeups/second`. A one-off `sudo powertop --calibrate`, on battery,
builds it. The arg-less `NOPASSWD` rule above deliberately does not cover
`--calibrate` — it prompts for a password, which is correct for a
once-ever command and not a reason to widen the rule.

Once installed, `battery.sh`'s "Where it goes" tooltip section (package /
uncore RAPL draw vs. the residual going to screen/disk/radios) appears
automatically — the module already degrades gracefully to just the top-level
draw number when `energy_uj` isn't readable, so a fresh checkout works before
this script has ever run.

## Power modes (`system/powermode/`)

Three modes. Left click on the battery pill still cycles just **full ↔ eco**;
**battery** is entered automatically by the AC cable and only ever left
automatically (plug back in, or the charge drops too low):

- **full** — CPU EPP/turbo/platform-profile at maximum, PCI/NVMe runtime PM
  left alone, waybar polls at its normal (config.jsonc) rate, tooltips rebuild
  on every poll.
- **battery** 🔋 — the cable comes out and nothing is asked to stop. CPU stays
  responsive (EPP `balance_power`, turbo on, ACPI profile one notch down —
  `PM_BAT_*`), but takes every I/O-side eco saving for free: PCI/NVMe runtime
  PM, `vm.laptop_mode`/writeback, WiFi power save, waybar's slower polling and
  cached tooltips. Backlight drops to `PM_BAT_BRIGHT` (70% by default).
  Nothing is paused, stopped, or unloaded. If the charge falls under
  `PM_BAT_ECO_PCT` (40% by default) — checked by battery-guard.sh's existing
  30s poll — it escalates itself to eco.
- **eco** 🌿 — CPU EPP/turbo/platform-profile at minimum, the same I/O-side
  savings as battery, backlight dropped further to `PM_ECO_BRIGHT` (40%
  default; restoring on the way back to full always returns the level from
  *before* the first dim, battery's included). This is the mode that actually
  frees things up. `docker`/frappe-bench containers get one of three outcomes
  rather than a blanket stop: a container with a live `docker exec`/`build`
  attached is **left running** (stopping it would kill whatever's mid-flight),
  one with no live docker command but a `PM_ECO_BUSY_PROCS` process running
  (`claude` by default) is **paused** (freezes the CPU cost, resumes
  instantly), and an idle one is **stopped**, same as before. Going back to
  full unpauses anything paused; stopped containers stay stopped, by design —
  start a bench on demand with `frappe-dev <env> up`.

**Eco drain sequence.** Entering eco pauses `hermes` at once (`SIGTSTP`, a
catchable stop signal — `SIGCONT` on the way back to full), then waits before
touching anything CPU-hungrier: docker's three-way decision above, a
`paseo chat post` telling you the machine is on battery, and unloading any
`ollama` model from RAM/VRAM (`ollama stop`, which idles the model without
stopping `ollama.service`) all happen together, but only once every open
`omp`/`pi` coding-agent session has gone quiet. "Quiet" is measured by
combined CPU ticks across matching processes every `PM_ECO_DRAIN_POLL`
seconds; under `PM_ECO_DRAIN_IDLE_CPU` seconds of CPU time in a window counts
as idle. A session that keeps working is never cut off — except battery
drain outranks it: below `PM_ECO_DRAIN_FORCE_PCT`, the wait ends regardless of
what omp/pi are doing. Matching for hermes and omp/pi is on the full command
line (`PM_ECO_HERMES_MATCH`, `PM_ECO_AGENT_MATCH`), not the process name —
omp runs under `bun`, pi under `node`, hermes under its venv's `python`, so
`ps`'s `comm` column can't tell one agent from any other tool sharing that
interpreter. Unloaded ollama model names are remembered; re-warming them on
the way back to full is opt-in (`PM_ECO_OLLAMA_RESTORE=1`, off by default —
reloading a multi-GB model on every cable plug costs more than one slow first
prompt after eco). Returning to full also cancels a wait still in progress.

**Paused** freezes userspace only — the container's kernel keeps servicing TCP
keepalives on every socket still open, so a paused bench and its redis go on
trading a few packets a second, and their veths show up as "busy" in powertop
(see the calibration note above; without a power model powertop ranks by raw
activity, and a veth floats to the top for that reason alone). Measured with
all containers paused: ~30 pkts/s total across six veths, `RetransSegs` flat
(keepalive traffic, not retransmits), against a system-wide NET_RX softirq
rate of 54.8/s — sub-milliwatt. **Stopped** containers don't do this, because
stopping tears the netns down.

A click sets a manual override that survives until the cable state changes
(unplugging or plugging back in always re-decides). The battery→eco
escalation also sets that override — once eco is reached on a draining
battery it stays there rather than flapping back to battery a percent later.
Independently, `battery-guard.sh`'s existing 30s poll also watches for a
**weak charger** — AC reports online but the battery is still discharging, or
a USB-C source negotiates less than `PM_WEAK_MIN_W` — and forces eco with a
critical notification until the charger recovers, overriding even a manual
full.

Every value — EPP, turbo, the profile, PCI PM, brightness, the battery→eco
threshold, the docker prefix, which processes count as "work in flight", the
eco drain sequence's matching patterns and thresholds, the weak-charger
threshold, waybar's poll intervals — lives in one file, `mango/powermode.conf`,
tracked and symlinked like everything else, with a comment over each key.
Editing it needs no reinstall; switching modes picks the new value up
immediately.

```
~/.config/mango/scripts/powermode.sh status   # current mode + why
~/.config/mango/scripts/powermode.sh eco      # force eco (sets the manual override)
~/.config/mango/scripts/powermode.sh full     # force full
~/.config/mango/scripts/powermode.sh battery  # force battery
~/.config/mango/scripts/powermode.sh low      # internal: battery -> eco escalation, sets the manual override
~/.config/mango/scripts/powermode.sh drain    # internal: the eco wait/pause loop, not for manual use
~/.config/mango/scripts/powermode.sh test     # decision-table self-check, no hardware touched
```

CPU EPP, turbo, the ACPI platform profile, PCI/NVMe runtime PM, snd_hda power
save and `vm.laptop_mode`/writeback are all root-owned sysfs, applied by
`/usr/local/bin/mango-powermode` — the only thing in this repo that writes any
of them. `powermode.conf` is user-writable, so that helper never sources it or
takes a path from it: `powermode.sh` resolves the config into `KEY=value`
lines for one target mode and pipes them in on stdin, and the helper
re-validates every value against a closed set before touching sysfs. Install
once:

```
sudo system/powermode/install.sh
```

Same shape as `system/rapl/`: sudo-gated, not symlinked, one fixed
`NOPASSWD` verb (`mango-powermode apply`, no free arguments — the payload
travels on stdin, which sudoers can't see, so the helper is the actual
boundary). `mango-powermode test` runs its own self-check against a fixture
`/sys` tree first and refuses to install the sudoers rule if it fails.

Before this is installed, mode switches still work — brightness, docker, and
waybar's poll intervals are all userspace — they just leave the CPU/PCI knobs
untouched, the same graceful-degradation shape as `battery.sh`'s RAPL section
before `system/rapl/install.sh` has run.

## Security note

No credentials are tracked here, and none should ever be. Secrets belong in
the KeePassXC keyring, which `mango/scripts/keyring-lookup.sh` reads at runtime
— `rofi/ai.sh` is the worked example.

Two things are untracked specifically because they hold machine-local values:
`fish/conf.d/claude.fish` (API keys exported into the shell) and
`git/config.local` (your name and email). Both are in `.gitignore`, alongside
`*.kdbx`/`*.key`/`*.pem`/`.env` catch-alls — this repo is symlinked into
`~/.config`, so an application can drop a new file into a tracked directory at
any time, and `git add -A` would otherwise sweep it up.

## Credits

**The design of this desktop originates with
[end-4/dots-hyprland](https://github.com/end-4/dots-hyprland) — the
"illogical-impulse" configuration — licensed GPL-3.0.** That project is the
source of the visual language and the colour architecture here, and deserves
the credit for both.

Ported from it, with modification:

| What | Adaptation |
|---|---|
| The whole matugen template set (`matugen/templates/*`) | retargeted at waybar/rofi/swaylock/mako instead of quickshell QML |
| `mango/scripts/switchwall.sh` | rewritten for mango: `swaybg` + `mmsg` instead of `hyprctl`; dropped video wallpaper, monitor queries, AI wallpaper categorization |
| The waybar visual design (`matugen/templates/waybar/style.css`) | reproduces its bar — opaque 40px, three centred rounded pills, matching fonts and warning colours — in GTK CSS rather than QML |
| The gruvbox ANSI seed (`[config.custom_colors]`) | its `scheme-base.json`, driven through matugen's `blend = true` instead of `generate_colors_material.py` |
| `mango/scripts/keyring-lookup.sh`, `vscode-set-color.sh` | its keyring and VS Code colour helpers, renamed into mango's namespace; keyring collection lookup fixed for KeePassXC |

Path names from that project (`quickshell/`, `illogical-impulse/`) have been
renamed to mango's own namespace because quickshell no longer runs on this
machine — waybar and rofi replaced it. The rename is organisational only and
changes nothing about the provenance above.

## Checklist — not covered by this repo yet

Things this desktop depends on that aren't tracked here:

- [ ] `systemctl enable --now atop.service atopacct.service` — the CPU tooltip's
      "recent peaks" section reads `/var/log/atop/atop_YYYYMMDD`, which only
      exists while these run (already enabled on this machine)
- [ ] `/usr/local/bin/keepassxc-stash-pw` + the SDDM PAM hook that feeds
      `/run/keepassxc-unlock/$USER`
- [ ] `systemd --user` masks for `gnome-keyring-daemon.{service,socket}` →
      `/dev/null` (lets KeePassXC own the Secret Service)
- [ ] `~/.config/autostart/*.desktop`
- [ ] `/etc/systemd/logind.conf.d/10-power.conf` — `HandlePowerKey=ignore`
      plus `HandlePowerKeyLongPress=ignore`, so `mango/scripts/powerkey.py` owns
      the power button. Without it logind suspends the instant the key goes
      down and the tap-for-menu tier is unreachable
- [ ] `/etc/UPower/UPower.conf` — `PercentageAction=2.0` with
      `CriticalPowerAction=Auto` is the backstop *below* `battery-guard.sh`.
      With hibernate unavailable (see below) `Auto` means power off, so if the
      guard is ever not running, 2% is an unclean session kill. Left as-is
      deliberately: it is the right last resort, it just should never be reached
- [ ] **Hibernate does not work on this machine** — the swapfile is smaller
      than RAM, `/sys/power/resume` is unset, and there is no `resume=` on the
      kernel cmdline. The wlogout Hibernate button is therefore dead, and
      `battery-guard.sh` suspends rather than hibernates. Enabling it needs a
      swapfile larger than RAM, `resume=`/`resume_offset=` on the cmdline, and
      the `resume` hook in `mkinitcpio.conf` — then the button works and the
      guard's action can be switched over
- [ ] `~/.config/environment.d/{claude,ssh-agent}.conf`
- [ ] `~/.config/environment.d/paseo.conf` — `PASEO_LISTEN=0.0.0.0:6767`, so the
      Paseo daemon is reachable over `wg_hetzner`; safe only because ufw's
      `deny incoming` restricts port 6767 to that interface (see `~/brain/firewall.md`)
- [ ] `~/.config/mimeapps.list` + custom `~/.local/share/applications/*.desktop`
- [ ] `dolphinrc`, `kiorc`, `filetypesrc`, `darklyrc`, `konsolerc`
- [ ] `gtk-3.0/bookmarks` (check for sensitive paths first)
- [ ] `{chrome,code}-flags.conf`
- [ ] KeePassXC: add attribute `application=mango` to the OpenRouter key entry
      (was `illogical-impulse`; Alt+I in rofi stays broken until this is done)
- [ ] a default wallpaper, or just document `~/Wallpapers/`
- [ ] `hypr/hyprlock.conf` — currently unused (swaylock is bound instead); include or delete
- [ ] `Code/User/settings.json` (`material-code.primaryColor`)
- [ ] fill in `~/.config/git/config.local` (name and email) — `install-config.sh`
      seeds it from `git/config.local.example`, but git refuses to commit until
      it has an identity
- [ ] delete `~/.gitconfig` after installing `git-delta`. It shadows the tracked
      `~/.config/git/config` — git reads XDG first and `$HOME` second, later
      wins — so none of the delta settings apply while it exists. Install delta
      *before* deleting it, or every paged git command fails with "cannot run
      delta"
- [ ] clean up untracked drift in `~/.config/fish/`: `auto-Hypr.fish` (dead —
      it lives in the fish root, not `conf.d/`, so nothing sources it, and it
      execs Hyprland anyway), plus five `.bak.20260802-*` files left by the
      install run. `conf.d/claude.fish` stays untracked on purpose (see below)
- [ ] move any API keys still exported from `conf.d/claude.fish` into the
      KeePassXC keyring, so `keyring-lookup.sh` is the only way they are read
- [ ] prune `~/.config/hypr/*.new`/`*.old` cruft
- [ ] confirm `cava`, `foot`, `fuzzel`, `hyprmonitor` are actually dead before discarding
- [ ] delete `~/.config/quickshell.old` (12 MB of the QML shell that waybar+rofi
      replaced — moved aside, not removed, in case something still reaches for it)
- [ ] delete `~/.local/state/quickshell/` — superseded by `~/.local/state/mango/`
- [ ] `matugen/templates/hyprland/*` writes into `~/.config/hypr/`, which mango
      never reads, and `matugen/templates/fuzzel/` themes a launcher rofi
      replaced — both probably dead, both harmless; confirm then delete
