#!/bin/bash
# Keep a local history of the KeePassXC database, and warn when a change
# looks like the rollback of 2026-09-09.
#
# Why: the Nextcloud Android app re-uploads a stale copy of the database.
# The laptop then downloads it over the good file. KeePassXC reloads without
# a word when it has no unsaved changes, so the loss is silent.
# See ~/brain/Areas/infra/hosts/arrakis.madmin.net/nextcloud.md, 2026-09-10.
set -uo pipefail

DB="$HOME/Nextcloud/Database/Consolidated.kdbx"
DIR="$HOME/.local/state/kdbx-history"
SYNCLOG="$HOME/.local/share/Nextcloud/Nextcloud_sync.log"
KPXC="$HOME/.config/keepassxc/keepassxc.ini"
KEEP=300
SHRINK_PCT=3        # a drop this large is the rollback signature (real ones were 11%)

say()  { echo "$*"; }
warn() { echo "$*" >&2; command -v notify-send >/dev/null &&
         notify-send -u critical "KeePassXC database" "$*" 2>/dev/null; }

is_kdbx() { [ "$(head -c4 "$1" | od -An -tx1 | tr -d ' \n')" = "03d9a29a" ]; }

newest() { ls -1t "$DIR"/Consolidated-*.kdbx 2>/dev/null | head -1; }

# Was the last transfer of this file a download? A download means the change
# came from the server, so this laptop did not make it.
came_from_server() {
  [ -r "$SYNCLOG" ] || return 1
  local last
  last=$(grep 'Database/Consolidated.kdbx' "$SYNCLOG" | tail -1)
  [ -n "$last" ] || return 1
  [ "$(echo "$last" | cut -d'|' -f5)" = "2" ]
}

archive() {
  [ -s "$DB" ] || exit 0
  is_kdbx "$DB" || { warn "The file is not a KeePassXC database. Not archived."; exit 0; }
  mkdir -p "$DIR"; chmod 700 "$DIR"

  local prev prev_size new new_size
  prev=$(newest)
  new="$DIR/Consolidated-$(date -r "$DB" +%Y-%m-%d_%H-%M-%S).kdbx"
  [ -e "$new" ] && exit 0
  install -m600 "$DB" "$new"

  if [ -n "$prev" ]; then
    prev_size=$(stat -c%s "$prev"); new_size=$(stat -c%s "$new")
    if [ "$new_size" -lt "$prev_size" ]; then
      local drop=$(( (prev_size - new_size) * 100 / prev_size ))
      if [ "$drop" -ge "$SHRINK_PCT" ]; then
        warn "The database shrank by ${drop}%, $prev_size to $new_size bytes. This looks like a rollback. The copy before it is $(basename "$prev")."
      fi
    fi
  fi

  came_from_server && warn "Another device wrote the database. The laptop downloaded it. Check the entries before you save."

  ls -1t "$DIR"/Consolidated-*.kdbx | tail -n +$((KEEP+1)) | xargs -r rm -f
  exit 0
}

check() {
  local fail=0
  ok()   { say "  ok    $*"; }
  bad()  { say "  FAIL  $*"; fail=1; }

  say "kdbx-history checks"

  systemctl --user is-active --quiet kdbx-history.path \
    && ok "the watch is running" || bad "the watch is not running: systemctl --user enable --now kdbx-history.path"

  local n; n=$(ls -1 "$DIR"/Consolidated-*.kdbx 2>/dev/null | wc -l)
  [ "$n" -gt 0 ] && ok "the history holds $n copies" || bad "the history is empty"

  local live; live="$DIR/Consolidated-$(date -r "$DB" +%Y-%m-%d_%H-%M-%S).kdbx"
  [ -e "$live" ] && ok "the live file is in the history" || bad "the live file is not in the history: systemctl --user start kdbx-history.service"

  if grep -q '^AutoReloadOnChange=false' "$KPXC" 2>/dev/null; then
    ok "KeePassXC asks before it accepts an outside change"
  else
    bad "KeePassXC reloads an outside change without asking. Turn off Settings, General, \"Automatically reload the database when modified externally\". This is what hid the loss of 2026-09-09."
  fi

  local downs last
  downs=$(awk -F'|' '/Database\/Consolidated.kdbx/ && $5==2' "$SYNCLOG" 2>/dev/null | wc -l)
  if [ "$downs" -eq 0 ]; then
    ok "no other device has written the database in this log"
  else
    last=$(awk -F'|' '/Database\/Consolidated.kdbx/ && $5==2 {t=$6} END{if(t) print strftime("%F %H:%M",t)}' "$SYNCLOG")
    bad "$downs download(s) in the current sync log, the last at $last. Another device still writes this file. Remove the local copy of Database/Consolidated.kdbx from the Nextcloud Android app."
  fi

  grep -q "^BackupBeforeSave=true" "$KPXC" 2>/dev/null \
    && ok "KeePassXC keeps a backup before each save" \
    || say "  note  KeePassXC keeps no backup before saving. The watch covers this, so it is optional."

  say ""
  [ "$fail" -eq 0 ] && say "All checks pass." || say "Some checks fail. See above."
  return "$fail"
}

case "${1:-}" in
  --check) check ;;
  "")      archive ;;
  *)       echo "usage: $(basename "$0") [--check]" >&2; exit 2 ;;
esac
