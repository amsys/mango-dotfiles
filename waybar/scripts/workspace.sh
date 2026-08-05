#!/bin/bash
# One mango tag, one waybar pill, with a hover tooltip listing what is on it.
#
# Nine custom modules instead of ext/workspaces because a per-tag tooltip needs
# a per-tag widget: waybar's ext/workspaces module has no `tooltip` option at
# all (waybar-wlr-workspaces(5)), so the only thing it can ever say is nothing.
#
# There is no live thumbnail here and there cannot be: waybar tooltips are text
# GTK windows, and wlroots screencopy only captures the composited output, so an
# off-screen tag has no frame to grab in the first place.
#
#   workspace.sh <1-9> [monitor]  custom/ws#N exec — emit JSON
#   workspace.sh --watch          custom/wswatch exec — block on mmsg, signal all nine
#   workspace.sh test             assert the renderer against canned IPC output
set -u

. "$(dirname "$0")/tooltip.sh"
. "$(dirname "$0")/watch.sh"

SIGNAL=20

# Render both mmsg documents into "class" on the first line and one tab-
# separated window per line after it. Kept as a function taking the combined
# JSON on stdin so the selftest can feed it canned input instead of a live
# compositor.
#
# Every monitor has its own nine tags, so which monitor this pill speaks for is
# an argument: waybar cannot tell a module which output its bar is on, so
# bars.sh bakes the name into the exec line when it generates the per-output
# bars. Empty means "first monitor in the document" — the fallback path where
# waybar was launched bare, without bars.sh.
render() { # tag-index, monitor (may be empty); reads {all_tags} then {clients}
	jq -rs --argjson n "$1" --arg mon "${2:-}" '
		(if $mon == "" then .[0].all_tags[0].monitor else $mon end)        as $m |
		(first(.[0].all_tags[] | select(.monitor == $m)).tags[]
		 | select(.index == $n))                                          as $t |
		[ .[1].clients[] | select(.tags | index($n))
		                 | select(.monitor == $m) ]                       as $c |
		(if   $t.is_urgent       then "urgent"
		 elif $t.is_active       then "active"
		 elif ($c | length) == 0 then "empty"
		 else                         "occupied" end),
		($c[] | [ (if .is_urgent then "!" elif .is_minimized then "_"
		           elif .is_focused then "*" else " " end),
		          (.appid // "?"),
		          ((.title // "") | .[0:44]) ] | @tsv)'
}

# Two passes so every title starts at the same x: the appid column is padded to
# this tag's widest appid. Padding is only true alignment in a monospace face,
# so mark+appid ships in F_MONO (see tooltip.sh) while the title keeps the
# proportional body font — narrower, and it has nothing to line up with.
winrows() { # TSV mark/appid/title on stdin -> row markup, one per line
	awk -F'\t' -v mono="$F_MONO" -v dim="$C_DIM" '
		function esc(s) {
			gsub(/&/, "\\&amp;", s); gsub(/</, "\\&lt;", s); gsub(/>/, "\\&gt;", s)
			return s
		}
		{ m[NR] = $1; a[NR] = $2; t[NR] = $3; if (length($2) > w) w = length($2) }
		END { for (i = 1; i <= NR; i++)
			# Pad first, escape second: entities lengthen the string, so padding
			# an escaped appid would misalign every row holding a & or a <.
			printf "<span font_family=\"%s\">%s %s</span>  <span foreground=\"%s\">%s</span>\n",
				mono, m[i], esc(sprintf("%-*s", w, a[i])), dim, esc(t[i]) }'
}

# ---------------------------------------------------------------- watch

if [ "${1:-}" = "--watch" ]; then
	# One watcher for all nine pills. mango's IPC_WATCH_ARRANGGE raises
	# ALL_TAGS and ALL_CLIENTS together on every view/tag/map/unmap/focus/
	# urgent event, so this single stream already covers everything the pills
	# draw — a second `watch all-clients` would only duplicate every wakeup.
	#
	# ponytail: no state file, no pidfile. Continuous waybar module, so waybar
	# owns the lifecycle and restart-interval respawns us if mango restarts.
	echo # empty text = module hidden
	watch_loop "mmsg watch all-tags"
	exit 0
fi

# ---------------------------------------------------------------- selftest

if [ "${1:-}" = "test" ]; then
	TAGS='{"all_tags":[{"monitor":"eDP-1","tags":[
		{"index":1,"is_active":false,"is_urgent":false,"client_count":2},
		{"index":2,"is_active":true,"is_urgent":false,"client_count":0},
		{"index":3,"is_active":false,"is_urgent":true,"client_count":1},
		{"index":4,"is_active":false,"is_urgent":false,"client_count":0}]},
		{"monitor":"DP-1","tags":[
		{"index":1,"is_active":true,"is_urgent":false,"client_count":1},
		{"index":2,"is_active":false,"is_urgent":false,"client_count":0},
		{"index":3,"is_active":false,"is_urgent":false,"client_count":0},
		{"index":4,"is_active":false,"is_urgent":false,"client_count":0}]}]}'
	CLIENTS='{"clients":[
		{"appid":"kitty","title":"vim x & y <z>","monitor":"eDP-1","tags":[1],"is_focused":true,"is_urgent":false,"is_minimized":false},
		{"appid":"firefox","title":"news","monitor":"eDP-1","tags":[1],"is_focused":false,"is_urgent":false,"is_minimized":true},
		{"appid":"slack","title":"ping","monitor":"eDP-1","tags":[3],"is_focused":false,"is_urgent":true,"is_minimized":false},
		{"appid":"mpv","title":"film","monitor":"DP-1","tags":[1],"is_focused":true,"is_urgent":false,"is_minimized":false}]}'
	feed() { printf '%s\n%s\n' "$TAGS" "$CLIENTS" | render "$1" "${2-}"; }

	[ "$(feed 1 | head -1)" = occupied ] || { echo "class occupied wrong"; exit 1; }
	[ "$(feed 2 | head -1)" = active ] || { echo "class active wrong"; exit 1; }
	[ "$(feed 3 | head -1)" = urgent ] || { echo "class urgent wrong"; exit 1; }
	[ "$(feed 4 | head -1)" = empty ] || { echo "class empty wrong"; exit 1; }
	# An active tag with clients still reads as active, not occupied.
	[ "$(feed 2 | wc -l)" -eq 1 ] || { echo "empty tag listed windows"; exit 1; }
	[ "$(feed 1 | wc -l)" -eq 3 ] || { echo "tag 1 window count wrong"; exit 1; }
	feed 1 | sed -n 2p | grep -q '^\*	kitty	vim x & y <z>$' || { echo "focused row wrong: $(feed 1 | sed -n 2p)"; exit 1; }
	feed 1 | sed -n 3p | grep -q '^_	firefox' || { echo "minimized mark wrong"; exit 1; }
	feed 3 | sed -n 2p | grep -q '^!	slack' || { echo "urgent mark wrong"; exit 1; }

	# Per-monitor: the same tag index is a different tag on each output, and a
	# client on the other screen is not this tag's business.
	[ "$(feed 1 eDP-1 | head -1)" = occupied ] || { echo "eDP-1 tag 1 class wrong"; exit 1; }
	[ "$(feed 1 DP-1 | head -1)" = active ] || { echo "DP-1 tag 1 class wrong"; exit 1; }
	[ "$(feed 3 DP-1 | head -1)" = empty ] || { echo "urgency must not cross monitors"; exit 1; }
	feed 1 DP-1 | grep -q mpv || { echo "DP-1 client missing"; exit 1; }
	feed 1 DP-1 | grep -q kitty && { echo "eDP-1 client leaked onto DP-1"; exit 1; }
	feed 1 eDP-1 | grep -q mpv && { echo "DP-1 client leaked onto eDP-1"; exit 1; }
	# No monitor argument keeps the first monitor whole — tags and clients from
	# the same output, not tags from one and clients from all of them.
	[ "$(feed 1 | wc -l)" = "$(feed 1 eDP-1 | wc -l)" ] || { echo "default monitor wrong"; exit 1; }
	# An unplugged monitor renders nothing rather than another screen's tags.
	[ -z "$(feed 1 HDMI-A-9)" ] || { echo "unknown monitor should render nothing"; exit 1; }
	# Titles carrying pango metacharacters must not reach the tooltip raw.
	printf '%s' 'vim x & y <z>' | esc | grep -q '^vim x &amp; y &lt;z&gt;$' || { echo "esc wrong"; exit 1; }

	# The appid column pads to the widest appid, so every title starts at one x.
	WR=$(printf '*\tkitty\tvim\n_\tfirefox\tnews\n' | winrows)
	printf '%s\n' "$WR" | sed -n 1p | grep -q '\* kitty  </span>' || { echo "appid pad wrong"; exit 1; }
	printf '%s\n' "$WR" | sed -n 2p | grep -q '_ firefox</span>' || { echo "widest appid should not pad"; exit 1; }
	# Padding is measured on the raw appid and applied before escaping, or every
	# row holding a metacharacter drifts by the length of its entity.
	printf '*\ta&b\ttt\n_\tlonger\tu\n' | winrows | sed -n 1p | grep -q 'a&amp;b   </span>' \
		|| { echo "pad must precede escape"; exit 1; }
	printf '*\tkitty\tx & y\n' | winrows | grep -q 'x &amp; y</span>$' || { echo "winrows title esc wrong"; exit 1; }
	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- module

N=${1:?usage: workspace.sh <1-9> [monitor] | --watch | test}
MON=${2-}

OUT=$({ mmsg get all-tags; mmsg get all-clients; } 2> /dev/null | render "$N" "$MON") || OUT=''
[ -n "$OUT" ] || { printf '{"text":"%s","class":"empty","tooltip":"Tag %s"}\n' "$N" "$N"; exit 0; }

CLASS=$(printf '%s\n' "$OUT" | head -1)
WINS=$(printf '%s\n' "$OUT" | tail -n +2)

# No "Tag N" title and no "Windows" section header: you know which pill you are
# hovering, so both were chrome above two lines of content. Dropping them also
# drops the `rule 44` that pinned the width, so the tooltip now sizes to its
# widest row — it can resize under the pointer when the focused window's title
# changes. Accepted: titles are capped at 44 chars in render(), so it is bounded.
TIP=$(
	if [ -z "$WINS" ]; then
		dim "empty"
	else
		# is_focused is per-tag in mango — two clients report it at once, one
		# per tag — so this marks *this tag's* focused window, not the globally
		# focused one.
		#
		# Piped through row() rather than indenting inside winrows so tooltip.sh
		# stays the single authority on row inset.
		printf '%s\n' "$WINS" | winrows | while IFS= read -r l; do row "$l"; done
	fi

	# ponytail: mango only re-announces on a title change for the focused
	# client, so a background window's title here can lag until the next real
	# event. Not worth a poll to fix.
)

emit "$CLASS" "$N" "$TIP"
