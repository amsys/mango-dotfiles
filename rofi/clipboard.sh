#!/usr/bin/env bash
# rofi script mode over cliphist: browse history, copy on Enter, delete, wipe,
# and show an inline thumbnail for entries that are images.
#
#   clipboard.sh --launch   spawn rofi with this mode and its keybinds
#   clipboard.sh            rofi script-mode callback
#   clipboard.sh test       assert the parsers against canned input
#
# Launched through --launch rather than a bare `rofi -show clipboard` because
# -kb-custom-N can only be set on the command line, and the delete and wipe
# keys are the whole point. The `clipboard:` mode stays registered in
# config.rasi so `rofi -show clipboard` still works from anywhere else.
set -euo pipefail

THEME="$HOME/.config/rofi/clipboard.rasi"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/rofi-clip"

# ponytail: head -500 over a ~90MB cliphist db; drop the cap if older entries
# are needed. Everything below inherits it, including the thumbnail sweep.
LIMIT=500

opt() { printf '\0%s\x1f%s\n' "$1" "$2"; }

# cliphist's preview for a non-text entry is "[[ binary data 12 KiB png 800x600 ]]"
# — size, kind, and dimensions when it knows them. Anchored both ends, so a
# pasted *description* of a binary entry is not mistaken for one.
# The pattern lives in a variable because a regex ending in "]]" would close
# the [[ ]] test itself.
BIN_RE='^\[\[ binary data ([0-9.]+ [KMGT]?i?B) ([^ ]+)( ([0-9]+x[0-9]+))? \]\]$'

# Echoes the extension, or nothing when the row is text or a binary we cannot
# render (a pdf has no thumbnail to decode).
img_ext() { # preview text
	[[ $1 =~ $BIN_RE ]] || return 0
	# Copied out first: the next =~ overwrites BASH_REMATCH.
	local kind=${BASH_REMATCH[2]}
	[[ $kind =~ ^(png|jpg|jpeg|gif|bmp|webp)$ ]] && printf '%s' "$kind"
	return 0
}

# What the row actually shows. Text passes through untouched; a binary entry
# becomes "png · 717×433 · 10 KiB", because the thumbnail beside it is already
# the preview — repeating "[[ binary data ]]" next to the image says nothing.
pretty() { # preview text
	[[ $1 =~ $BIN_RE ]] || { printf '%s' "$1"; return 0; }
	local size=${BASH_REMATCH[1]} kind=${BASH_REMATCH[2]} dim=${BASH_REMATCH[4]}
	if [[ -n $dim ]]; then
		printf '%s · %s · %s' "$kind" "${dim/x/×}" "$size"
	else
		printf '%s · %s' "$kind" "$size"
	fi
}

# "" -> armed (ask again), armed -> wiped. Split out so the guard is testable
# without a live clipboard.
confirm_wipe() { [[ $1 == armed ]] && printf 'wiped' || printf 'armed'; }

# Decode one image entry into the cache and echo its path. Keyed on cliphist's
# id, which is stable for the life of an entry, so each image decodes once.
thumb() { # id, ext, full-row
	local path="$CACHE/$1.$2"
	if [[ ! -s $path ]]; then
		cliphist decode <<< "$3" > "$path.tmp" 2> /dev/null && mv "$path.tmp" "$path" || {
			rm -f "$path.tmp"
			return 1
		}
	fi
	printf '%s' "$path"
}

# Drop cached thumbnails whose entry is gone. cliphist caps the db at 750, so
# without this the cache grows without bound as entries roll off the end.
prune() { # ids currently listed, one per line
	local live=$1 f id
	for f in "$CACHE"/*; do
		[[ -e $f ]] || continue
		id=${f##*/}
		id=${id%%.*}
		grep -qxF "$id" <<< "$live" || rm -f "$f"
	done
}

# ---------------------------------------------------------------- selftest

if [[ ${1:-} == test ]]; then
	[[ $(img_ext '[[ binary data 12 KiB png 800x600 ]]') == png ]] || { echo "png not detected"; exit 1; }
	[[ $(img_ext '[[ binary data 1 MiB jpeg 4000x3000 ]]') == jpeg ]] || { echo "jpeg not detected"; exit 1; }
	[[ -z $(img_ext 'some copied text') ]] || { echo "text misread as image"; exit 1; }
	[[ -z $(img_ext 'see [[ binary data 1 KiB png 1x1 ]] in the docs') ]] || { echo "prose misread as image"; exit 1; }
	[[ -z $(img_ext '[[ binary data 3 KiB application/pdf ]]') ]] || { echo "pdf offered a thumbnail"; exit 1; }
	[[ $(pretty '[[ binary data 10 KiB png 717x433 ]]') == 'png · 717×433 · 10 KiB' ]] || { echo "image label wrong"; exit 1; }
	[[ $(pretty '[[ binary data 3 KiB application/pdf ]]') == 'application/pdf · 3 KiB' ]] || { echo "non-image label wrong"; exit 1; }
	[[ $(pretty 'git push --force-with-lease') == 'git push --force-with-lease' ]] || { echo "text label rewritten"; exit 1; }
	[[ $(confirm_wipe '') == armed ]] || { echo "wipe armed wrongly"; exit 1; }
	[[ $(confirm_wipe armed) == wiped ]] || { echo "wipe did not confirm"; exit 1; }
	tmp=$(mktemp -d)
	CACHE=$tmp
	touch "$tmp/11.png" "$tmp/22.png"
	prune $'11\n33'
	[[ -e $tmp/11.png && ! -e $tmp/22.png ]] || { echo "prune kept or dropped the wrong file"; exit 1; }
	rm -rf "$tmp"
	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- launcher

if [[ ${1:-} == --launch ]]; then
	# Alt+d / Alt+Shift+d match ai.sh's delete / wipe-everything pair. Check
	# mango/config.conf before picking any other Alt combo: a compositor bind
	# never reaches rofi.
	# Size the window to the widest preview instead of a fixed width. -theme-str
	# is the only way in: the width has to be settled before rofi starts and the
	# mode script does not run until after. wc -L, not awk length(), because it
	# counts display width — an entry full of "…" must not overcount.
	# ponytail: cliphist truncates previews at 100 chars, so a full history
	# usually lands on the upper clamp; this earns its keep on a short one.
	W=$(($(cliphist list 2> /dev/null | head -"$LIMIT" | cut -f2- | wc -L) + 12))
	((W < 55)) && W=55   # the message line's hotkey legend is ~55 wide
	((W > 118)) && W=118 # ~830px at font 11, comfortable on a 1920px screen

	exec rofi -show clipboard -theme "$THEME" \
		-theme-str "window { width: ${W}ch; }" \
		-kb-custom-1 "Alt+d" \
		-kb-custom-2 "Alt+Shift+d"
fi

# ---------------------------------------------------------------- mode

mkdir -p "$CACHE"

# ROFI_DATA is echoed back to us on every call, which is where the wipe's
# armed/disarmed state lives — no state file for something that must not
# outlive the menu.
STATE="${ROFI_DATA:-}"
NOTE=""

case "${ROFI_RETV:-0}" in
1) # Enter on a row — the whole row goes back to cliphist, id and all
	cliphist decode <<< "$1" | wl-copy
	exit 0
	;;
10) # Alt+d — delete the highlighted entry, then fall through and re-render
	[[ -n ${1:-} ]] && cliphist delete <<< "$1"
	STATE=""
	NOTE="deleted"
	;;
11) # Alt+Shift+d — wipe, but only on the second consecutive press
	if [[ $(confirm_wipe "$STATE") == wiped ]]; then
		cliphist wipe
		rm -f "$CACHE"/*
		STATE=""
		NOTE="history cleared"
	else
		STATE="armed"
		NOTE="⚠  press Alt+Shift+D again to clear the whole history"
	fi
	;;
esac

LIST=$(cliphist list 2> /dev/null | head -"$LIMIT" || true)
prune "$(cut -f1 <<< "$LIST")"

opt no-custom true
opt use-hot-keys true
opt keep-selection true
if [[ -n $NOTE ]]; then
	opt message "$NOTE"
else
	# "shown", not "entries": the list is capped at $LIMIT and the db holds more.
	opt message "$(grep -c . <<< "$LIST") shown   —   Alt+d deletes · Alt+Shift+D clears all"
fi
opt data "$STATE"

# Rows go through a temp file, not a variable: they carry NUL bytes (rofi's
# row/option separator) and $(...) silently drops those.
#
# The row text stays "<id>\t<preview>" and only the `display` option changes
# what is drawn, which is what keeps the id off the screen without taking it
# away from the handlers above: $1 is still the whole row on both the Enter and
# the Alt+d path. (The `info` option would put the pretty text in the row
# instead, but rofi only promises ROFI_INFO on a *selection* — Alt+d is a custom
# keybinding, so delete would lose the id.)
BUF=$(mktemp)
trap 'rm -f "$BUF"' EXIT
while IFS= read -r row; do
	[[ -n $row ]] || continue
	preview=${row#*$'\t'}
	disp=$(pretty "$preview")
	ext=$(img_ext "$preview")
	if [[ -n $ext ]] && path=$(thumb "${row%%$'\t'*}" "$ext" "$row"); then
		printf '%s\0icon\x1f%s\x1fdisplay\x1f%s\n' "$row" "$path" "$disp" >> "$BUF"
	else
		printf '%s\0display\x1f%s\n' "$row" "$disp" >> "$BUF"
	fi
done <<< "$LIST"
cat "$BUF"
