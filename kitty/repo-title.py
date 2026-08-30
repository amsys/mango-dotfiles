# kitty watcher: prefix every window title with its repo label, so windows
# in different ~/work projects are distinguishable at a glance.
#
# Label rule (kept in sync with mango/scripts/claude-notify.sh — see that
# file if you change this):
#   ~/work/<repo>/...            -> <repo>
#   ~/work/<repo>/src/<sub>/...  -> <repo>/<sub>   (superrepo wrapper shape)
#   not under ~/work             -> no label, title left untouched

from pathlib import Path
from typing import Any

WORK = Path.home() / "work"
SEP = " · "  # " · "


def repo_label(cwd: str) -> str | None:
    try:
        rel = Path(cwd).resolve().relative_to(WORK)
    except ValueError:
        return None
    parts = rel.parts
    if not parts:
        return None
    if len(parts) >= 3 and parts[1] == "src":
        return f"{parts[0]}/{parts[2]}"
    return parts[0]


def on_title_change(boss: Any, window: Any, data: dict[str, Any]) -> None:
    # Only react to titles the child process set — set_title() below re-fires
    # this watcher with from_child=False, so skipping that branch is what
    # keeps this from recursing into itself.
    if not data.get("from_child"):
        return
    try:
        label = repo_label(window.cwd_of_child)
    except Exception:
        label = None
    title = data.get("title") or ""
    window.set_title(f"{label}{SEP}{title}" if label else None)
