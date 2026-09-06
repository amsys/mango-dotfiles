# Keybinds and launchers

`SUPER+/` opens the generated cheat sheet. `keybinds-cheatsheet.py` reads
`mango/config.conf`. For this reason the sheet is never out of date.

## The modifier rule

| Modifier | Owns |
|---|---|
| `ALT` | the applications: focus, launch, kill and tag assignment |
| `SUPER` | the desktop: movement between tags, movement of windows, the session |
| `CTRL` | **nothing.** It stays with the applications. |

A compositor bind never reaches the focused application. A bind on plain
`CTRL` therefore takes that key from every program at the same time. This
includes word-jump, select-by-word and browser tab switching. `Ctrl+Print`
is the one exception, because `Print` is not a text-editing key.

## Binds to know

| Key | Function |
|---|---|
| `Alt+Return` | terminal (kitty) |
| `Alt+Space` | app launcher |
| `Alt+V` | clipboard history (`rofi/clipboard.sh`). It shows inline image thumbnails. `Alt+D` deletes an entry |
| `Alt+I` | AI chat (`rofi/ai.sh`, see below) |
| ``Alt+` `` | window switcher (`rofi/window.sh`). It lists every window on every tag, and it includes the minimized windows |
| ``SUPER+` `` | jump back to the previous window |
| `Alt+Tab` | mango's jump labels. This is not an alt-tab switcher |
| `SUPER+W` | wallpaper switch (see [theming.md](theming.md)) |
| `SUPER+L` | lock with `sleep-lock.py lock`. It starts swaylock. It pauses the pomodoro while the screen stays locked |
| `SUPER+SHIFT+L` | suspend |
| `CTRL+ALT+Delete` | power menu |
| `Alt+H` | hotspot menu |
| `Alt+Shift+O` | OCR a region to the clipboard |
| `SUPER+SHIFT+T` / `+N` / `+R` | pomodoro: name the task / parking-lot note / weekly review |
| `SUPER+SHIFT+M` | mute the pomodoro chime |
| `SUPER+SHIFT+Escape` | kill a stuck rofi (`pkill -x rofi`). This is the manual escape. See the Pomodoro section of [bar.md](bar.md) |
| `SUPER+CTRL+Next` / `+Prior` | remote access: pull the next or the previous tag onto the VNC virtual output (`remote.sh --pull-next`/`--pull-prev`, see [bar.md](bar.md)) |
| `SUPER+CTRL+Home` | remote access: send the pulled tag back (`remote.sh --restore`) |

`SUPER+CTRL+Next`, `+Prior` and `+Home` stay on `SUPER`, not on bare
`CTRL`. The modifier rule above still applies. `SUPER+CTRL` already moves
windows (`movewin`). PgUp, PgDown and Home are unbound, letters aside. A
headless VNC client sends only the modifiers plus Tab, Esc, PgUp, PgDown
and Home. It must reach these three binds without a letter key.

These three keybinds are the path without a mouse. With a mouse, the
remote-control strip on the headless bar does the same pulls and the same
restore by click. See the note on the `HEADLESS-*` bar in
[bar.md](bar.md).

## Screenshots

All screenshots go through `mango/scripts/screenshot.sh`. For this reason
the save directory stays in one place (`MANGO_SCREENSHOT_DIR`).

| Key | Function |
|---|---|
| `Print` | all outputs, clipboard only |
| `Alt+S` / `Ctrl+Print` | active monitor, save + copy |
| `SUPER+Print` | focused window, save + copy |
| `Alt+Shift+S` | region, save + copy |
| `Shift+Print` | region, annotate in swappy |

The "active monitor" comes from the selected-monitor flag of the
compositor. It does not come from a guess at the cursor position.

## Screen recording

| Key | Function |
|---|---|
| `Alt+Print` | record a region; press again to stop |
| `Alt+Shift+Print` | record the whole output; press again to stop |

`wf-recorder` has no toggle of its own, so `screenrecord.sh` controls both
ends. `screenrecord.sh` stops the recorder with `SIGINT`. wf-recorder
traps `SIGINT` and flushes the file. A recording stopped in a different
way does not play. The output goes to `~/Videos/Recordings/`.

## The window switcher

The built-in `window` mode of rofi works on X11 only. `rofi/window.sh`
builds the list from the compositor (`mmsg get all-clients`). It focuses a
window with one dispatch. That dispatch views the tag of the window,
restores the window and focuses it. The list includes the minimized
windows on purpose. This is the only way back to a specific one.

## AI chat (`Alt+I`)

`rofi/ai.sh` is a chat, not a box for one question. The list view shows
the transcript. Each follow-up sends the full conversation.

| Key | Function |
|---|---|
| `Enter` | send |
| `Ctrl+Enter` | copy the selected message |
| `Alt+N` | new chat |
| `Alt+[` / `Alt+]` | previous / next conversation |
| `Alt+D` / `Alt+Shift+D` | clear chat / wipe all history |
| `Alt+R` | refresh: show an answer that has arrived |

The request runs detached, so the window stays live. The script protocol
of rofi has no timer. As a result, the transcript does not refresh itself.
Press `Alt+R` when the answer arrives. The conversations are in
`~/.local/share/rofi-ai/`, capped at 20 chats of 40 messages.

The requests go to OpenRouter. `MANGO_AI_MODEL` selects a different model.
The API key comes from the KeePassXC Secret Service. The key entry needs
the attribute `application=mango` (Entry → Advanced → Additional
attributes). The caption line shows all error states: a locked keyring, a
missing entry or a failed call. A failed call removes the question from
the transcript.
