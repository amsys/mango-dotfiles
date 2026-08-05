#!/usr/bin/env python3
"""Parse ~/.config/mango/config.conf keybinds into a standalone HTML cheatsheet.

Usage:
    keybinds-cheatsheet.py          # regenerate ~/.config/mango/keybinds.html
    keybinds-cheatsheet.py --open   # regenerate, then xdg-open it
"""
import html
import json
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path

CONFIG = Path.home() / ".config/mango/config.conf"
OUTPUT = Path.home() / ".config/mango/keybinds.html"
PALETTE = Path.home() / ".local/state/mango/generated/colors.json"

BIND_RE = re.compile(r"^(bind|mousebind|axisbind)=(.*)$")

MOD_ORDER = ["SUPER", "CTRL", "ALT", "SHIFT"]
MOD_LABEL = {"SUPER": "Super", "CTRL": "Ctrl", "ALT": "Alt", "SHIFT": "Shift"}

KEY_LABEL = {
    "return": "Enter", "space": "Space", "slash": "/", "backslash": "\\",
    "equal": "=", "tab": "Tab", "print": "PrtSc",
    "left": "←", "right": "→", "up": "↑", "down": "↓",
    "btn_left": "Left Click", "btn_right": "Right Click", "btn_middle": "Middle Click",
    "mouse_up": "Scroll Up", "mouse_down": "Scroll Down",
    "xf86audioraisevolume": "Vol +", "xf86audiolowervolume": "Vol -",
    "xf86audiomute": "Mute", "xf86audiomicmute": "Mic Mute",
    "xf86monbrightnessup": "Bright +", "xf86monbrightnessdown": "Bright -",
    "xf86audioplay": "Play/Pause", "xf86audionext": "Next Track", "xf86audioprev": "Prev Track",
    "delete": "Delete",
}

# Plain action -> description. Deliberately ignores args/key: the key chip
# already shows the number or arrow, so repeating "tag 3" / "left" in the
# description text would just make every row's grouping key unique and
# defeat collapsing (see collapse()).
ACTION_DESC = {
    "reload_config": "Reload mango config",
    "spawn": None,  # derived from command
    "spawn_shell": None,
    "toggle_named_scratchpad": "Toggle named scratchpad",
    "quit": "Quit session",
    "killclient": "Close window",
    "focusstack": "Focus next window",
    "focusdir": "Focus window in direction",
    "exchange_client": "Swap window in direction",
    "toggleglobal": "Toggle global tag",
    "togglejump": "Toggle jump/swap window",
    "togglefloating": "Toggle floating",
    "togglemaximizescreen": "Toggle maximize",
    "togglefullscreen": "Toggle fullscreen",
    "togglefakefullscreen": "Toggle fake fullscreen",
    "minimized": "Minimize window",
    "toggleoverlay": "Toggle overlay",
    "restore_minimized": "Restore minimized window",
    "toggle_scratchpad": "Toggle scratchpad",
    "set_proportion": "Reset window proportion",
    "switch_proportion_preset": "Cycle proportion preset",
    "scroller_stack": "Move stack in direction",
    "dwindle_toggle_split_direction": "Toggle split direction",
    "switch_layout": "Switch layout",
    "viewtoleft": "Switch to tag left",
    "viewtoleft_have_client": "Switch to tag left (with client)",
    "viewtoright": "Switch to tag right",
    "viewtoright_have_client": "Switch to tag right (with client)",
    "tagtoleft": "Move tag left",
    "tagtoright": "Move tag right",
    "view": "Switch to tag",
    "tag": "Move window to tag",
    "focusmon": "Focus monitor",
    "tagmon": "Move window to monitor",
    "togglegaps": "Toggle gaps",
    "movewin": "Move window (50px steps)",
    "resizewin": "Resize window (50px steps)",
}

# spawn command first-word -> friendly name
CMD_FRIENDLY = {
    "dolphin": "File manager",
    "kitty": "Terminal",
    "hyprpicker": "Color picker → clipboard",
    "wlogout": "Session menu",
}


def humanize_key(key):
    return KEY_LABEL.get(key.lower(), key)


def humanize_mods(mod_str):
    if mod_str.lower() in ("none", ""):
        return []
    parts = [p.upper() for p in mod_str.split("+")]
    ordered = [m for m in MOD_ORDER if m in parts]
    ordered += [p for p in parts if p not in MOD_ORDER]  # keep unknowns
    return [MOD_LABEL.get(m, m.title()) for m in ordered]


def describe_command(cmd):
    first = cmd.strip().split()[0] if cmd.strip() else cmd
    first = first.rsplit("/", 1)[-1]  # strip path
    friendly = CMD_FRIENDLY.get(first)
    return friendly or f"Run {first}"


def describe_action(action, args):
    # A few actions are ambiguous without their arg (which key/direction
    # doesn't already convey it) - special-case those before falling back
    # to the plain static description.
    if action == "incgaps" and args:
        try:
            return "Increase gaps" if float(args[0]) > 0 else "Decrease gaps"
        except ValueError:
            pass
    if action == "moveresize" and args:
        return {"curmove": "Move window (drag)", "curresize": "Resize window (drag)"}.get(
            args[0], "Drag"
        )
    return ACTION_DESC.get(action)  # None => spawn/spawn_shell, or unknown action falls through


def parse_line(kind, rest):
    """Return (mods, key, action, args, command_or_None)."""
    # bind, mousebind and axisbind all share the mods,key,action[,args...] shape
    parts = rest.split(",", 2)
    mods, key = parts[0], parts[1]
    remainder = parts[2] if len(parts) > 2 else ""

    if not remainder:
        return mods, key, "", [], None

    action_parts = remainder.split(",", 1)
    action = action_parts[0]
    tail = action_parts[1] if len(action_parts) > 1 else ""

    if action in ("spawn", "spawn_shell"):
        return mods, key, action, [], tail  # tail is the whole command, commas intact

    args = [a for a in tail.split(",") if a != ""] if tail else []
    return mods, key, action, args, None


def parse_config(path):
    """Returns list of (section, mods_list, key, description, command_or_None)."""
    rows = []
    section = "Uncategorized"
    pending_comment = None
    parsed_count = 0

    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line:
            # A blank line ends whatever comment block came before it - if it
            # was never attached to a bind, drop it rather than let it leak
            # into the next section as a stale title.
            pending_comment = None
            continue
        if line.startswith("#"):
            comment = line.lstrip("#").strip()
            # Keep the FIRST line of a contiguous comment block as the title,
            # not the last - a multi-line block like
            #   # Mouse Button Bindings
            #   # btn_left and btn_right can't bind none mod key
            # should title as "Mouse Button Bindings", and a commented-out
            # example bind at the end of a block shouldn't become the title.
            if comment and pending_comment is None:
                pending_comment = comment
            continue

        m = BIND_RE.match(line)
        if not m:
            continue
        kind, rest = m.groups()
        parsed_count += 1

        if pending_comment:
            section = pending_comment
        pending_comment = None

        mods_raw, key_raw, action, args, command = parse_line(kind, rest)
        mods = humanize_mods(mods_raw)
        key = humanize_key(key_raw)

        if command is not None:
            desc = describe_command(command)
        else:
            desc = describe_action(action, args) or action

        rows.append((section, mods, key, desc, command))

    return rows, parsed_count


DIGIT_RE = re.compile(r"^\d+$")
ARROWS = ["←", "↑", "↓", "→"]


def collapse(rows):
    """Collapse same-section/mods/desc/command rows whose keys are a digit run or the 4 arrows."""
    groups = {}
    order = []
    for section, mods, key, desc, command in rows:
        gkey = (section, tuple(mods), desc, command)
        if gkey not in groups:
            groups[gkey] = []
            order.append(gkey)
        groups[gkey].append(key)

    collapsed = []
    for gkey in order:
        section, mods, desc, command = gkey
        keys = groups[gkey]
        if len(keys) >= 3 and all(DIGIT_RE.match(k) for k in keys):
            nums = sorted(int(k) for k in keys)
            label = f"{nums[0]}…{nums[-1]}"
        elif len(keys) == 4 and set(keys) == set(ARROWS):
            label = " ".join(ARROWS)
        else:
            for k in keys:
                collapsed.append((section, list(mods), k, desc, command))
            continue
        collapsed.append((section, list(mods), label, desc, command))
    return collapsed


DEFAULT_PALETTE = {
    "background": "#131318", "surface": "#131318", "on_surface": "#e4e1e9",
    "on_surface_variant": "#c6c5d0", "primary": "#c2c1ff", "on_primary": "#2d2f6a",
    "primary_container": "#434578", "outline": "#8f8e99", "outline_variant": "#47464f",
}


def load_palette():
    try:
        data = json.loads(PALETTE.read_text())
        if isinstance(data, dict) and "background" in data:
            return {**DEFAULT_PALETTE, **data}
    except Exception:
        pass
    return DEFAULT_PALETTE


def render_html(rows, palette):
    sections = {}
    order = []
    for section, mods, key, desc, command in rows:
        if section not in sections:
            sections[section] = []
            order.append(section)
        sections[section].append((mods, key, desc, command))

    css = f"""
    :root {{
      --bg: {palette['background']}; --surface: {palette['surface']};
      --on-surface: {palette['on_surface']}; --muted: {palette['on_surface_variant']};
      --primary: {palette['primary']}; --on-primary: {palette['on_primary']};
      --chip: {palette['primary_container']}; --outline: {palette['outline_variant']};
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0; padding: 2rem; background: var(--bg); color: var(--on-surface);
      font-family: system-ui, sans-serif; line-height: 1.4;
    }}
    h1 {{ margin: 0 0 .25rem; font-size: 1.6rem; }}
    .meta {{ color: var(--muted); font-size: .85rem; margin-bottom: 1.5rem; }}
    .grid {{
      display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
      gap: 1rem;
    }}
    .card {{
      background: var(--surface); border: 1px solid var(--outline); border-radius: 10px;
      padding: 1rem 1.1rem;
    }}
    .card h2 {{
      margin: 0 0 .6rem; font-size: 1rem; text-transform: capitalize; color: var(--primary);
    }}
    table {{ width: 100%; border-collapse: collapse; }}
    tr {{ border-top: 1px solid var(--outline); }}
    tr:first-child {{ border-top: none; }}
    td {{ padding: .35rem 0; vertical-align: top; }}
    td.keys {{ white-space: nowrap; width: 1%; padding-right: .8rem; }}
    kbd {{
      display: inline-block; background: var(--chip); color: var(--on-primary);
      border-radius: 4px; padding: .1rem .45rem; font-size: .82rem; font-family: inherit;
      margin-right: 2px;
    }}
    .desc {{ font-size: .92rem; }}
    .cmd {{
      display: block; color: var(--muted); font-size: .78rem; font-family: monospace;
      margin-top: 2px; word-break: break-all;
    }}
    footer {{ margin-top: 2rem; color: var(--muted); font-size: .78rem; }}
    @media print {{
      body {{ background: white; color: black; padding: .5rem; }}
      .card {{ break-inside: avoid; border-color: #ccc; }}
      kbd {{ background: #eee; color: black; }}
    }}
    """

    def render_keys(mods, key):
        chips = "".join(f"<kbd>{html.escape(m)}</kbd>" for m in mods)
        chips += f"<kbd>{html.escape(key)}</kbd>"
        return chips

    body_sections = []
    for section in order:
        rows_html = []
        for mods, key, desc, command in sections[section]:
            cmd_html = f'<span class="cmd">{html.escape(command)}</span>' if command else ""
            rows_html.append(
                f'<tr><td class="keys">{render_keys(mods, key)}</td>'
                f'<td class="desc">{html.escape(desc)}{cmd_html}</td></tr>'
            )
        body_sections.append(
            f'<div class="card"><h2>{html.escape(section)}</h2>'
            f'<table>{"".join(rows_html)}</table></div>'
        )

    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M")
    return f"""<!doctype html>
<html><head><meta charset="utf-8">
<title>mango keybinds</title>
<style>{css}</style>
</head><body>
<h1>mango keybinds</h1>
<div class="meta">Generated {timestamp} from {html.escape(str(CONFIG))}</div>
<div class="grid">{"".join(body_sections)}</div>
<footer>Regenerate: {html.escape(str(Path.home()))}/.config/mango/scripts/keybinds-cheatsheet.py</footer>
</body></html>
"""


def main():
    rows, parsed_count = parse_config(CONFIG)
    collapsed = collapse(rows)
    print(f"parsed {parsed_count} binds -> {len(collapsed)} rows", file=sys.stderr)

    palette = load_palette()
    OUTPUT.write_text(render_html(collapsed, palette))

    if "--open" in sys.argv:
        subprocess.run(["xdg-open", str(OUTPUT)])


if __name__ == "__main__":
    main()
