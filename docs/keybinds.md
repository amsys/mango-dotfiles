# Keybinds and launchers

`SUPER+/` opens the generated cheat sheet. `keybinds-cheatsheet.py` parses
`mango/config.conf`, so the sheet is never out of date.

## The modifier rule

| Modifier | Owns |
|---|---|
| `ALT` | applications — focus, launch, kill, tag assignment |
| `SUPER` | the desktop — tag navigation, moving windows, session |
| `CTRL` | **nothing.** It stays with the applications. |

A compositor bind never reaches the focused application. A bind on plain
`CTRL` therefore takes that key away from every program at once (word-jump,
select-by-word, browser tab switching). `Ctrl+Print` is the one exception,
and `Print` is not a text-editing key.

## Binds to know

| Key | Function |
|---|---|
| `Alt+Return` | terminal (kitty) |
| `Alt+Space` | app launcher |
| `Alt+V` | clipboard history (`rofi/clipboard.sh`) — inline image thumbnails, `Alt+D` deletes |
| `Alt+I` | AI chat (`rofi/ai.sh`, see below) |
| ``Alt+` `` | window switcher (`rofi/window.sh`) — every window on every tag, minimized included |
| ``SUPER+` `` | jump back to the previous window |
| `Alt+Tab` | mango's jump labels — not an alt-tab switcher |
| `SUPER+W` | wallpaper switch (see [theming.md](theming.md)) |
| `SUPER+L` | lock (swaylock) |
| `SUPER+SHIFT+L` | suspend |
| `CTRL+ALT+Delete` | power menu |
| `Alt+H` | hotspot menu |
| `Alt+Shift+O` | OCR a region to the clipboard |
| `SUPER+SHIFT+T` / `+N` / `+R` | pomodoro: name task / parking-lot note / weekly review |
| `SUPER+SHIFT+M` | mute the pomodoro chime |
| `SUPER+CTRL+Next` / `+Prior` | remote access: pull the next/previous tag onto the VNC virtual output (`remote.sh --pull-next`/`--pull-prev`, see [bar.md](bar.md)) |
| `SUPER+CTRL+Home` | remote access: send the pulled tag back (`remote.sh --restore`) |

`SUPER+CTRL+Next`/`+Prior`/`+Home` stay on `SUPER`, not bare `CTRL` — the
modifier rule above still holds. `SUPER+CTRL` is already used for window
nudges (`movewin`); PgUp/PgDown/Home are unbound letters-aside, and a
headless VNC client that only sends modifiers plus Tab/Esc/PgUp/PgDown/Home
needs to reach these three without a letter key.

## Screenshots

All go through `mango/scripts/screenshot.sh`, so the save directory lives in
one place (`MANGO_SCREENSHOT_DIR`).

| Key | Function |
|---|---|
| `Print` | all outputs, clipboard only |
| `Alt+S` / `Ctrl+Print` | active monitor, save + copy |
| `SUPER+Print` | focused window, save + copy |
| `Alt+Shift+S` | region, save + copy |
| `Shift+Print` | region, annotate in swappy |

"Active monitor" comes from the compositor's own selected-monitor flag, not
from a cursor guess.

## Screen recording

| Key | Function |
|---|---|
| `Alt+Print` | record a region; press again to stop |
| `Alt+Shift+Print` | record the whole output; press again to stop |

`wf-recorder` has no toggle of its own, so `screenrecord.sh` wraps both
ends. It stops the recorder with `SIGINT` — wf-recorder traps it to flush
the file, and a recording killed any other way does not play. Output lands
in `~/Videos/Recordings/`.

## The window switcher

rofi's built-in `window` mode is X11-only. `rofi/window.sh` builds the list
from the compositor (`mmsg get all-clients`) and focuses with one dispatch,
which views the window's tag, restores it, and focuses it. Minimized
windows are listed on purpose — this is the only way back to a specific
one.

## AI chat (`Alt+I`)

`rofi/ai.sh` is a chat, not a one-shot question box. The list view is the
transcript; follow-ups carry the whole conversation.

| Key | Function |
|---|---|
| `Enter` | send |
| `Ctrl+Enter` | copy the selected message |
| `Alt+N` | new chat |
| `Alt+[` / `Alt+]` | previous / next conversation |
| `Alt+D` / `Alt+Shift+D` | clear chat / wipe all history |
| `Alt+R` | refresh — pull in an answer that has landed |

The request runs detached, so the window stays live. rofi's script protocol
has no timer, so the transcript does not refresh itself: press `Alt+R` when
the answer lands. Conversations live in `~/.local/share/rofi-ai/`, capped
at 20 chats of 40 messages.

Requests go to OpenRouter (`MANGO_AI_MODEL` overrides the model). The API
key comes from the KeePassXC Secret Service: the key entry needs the
attribute `application=mango` (Entry → Advanced → Additional attributes).
All error states (locked keyring, missing entry, failed call) show in the
caption line, and a failed call removes the question from the transcript.
