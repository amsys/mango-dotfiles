# Shared tooltip vocabulary for the waybar modules that emit JSON.
#
# Sourced, never executed — no shebang, POSIX only (net.sh is /bin/sh).
# Extracted from net.sh once cpu/memory/battery/clock needed the same meters;
# claudebar (a Rust binary) draws the same shapes from its own copy.
#
#   . "$(dirname "$0")/tooltip.sh"
#
# Every tooltip built from these reads as one system: bold blue title, a rule,
# then sections of `  <icon>  LABEL` followed by indented rows.

# Deliberately hardcoded one-dark constants rather than matugen colors, so the
# tooltips stay legible against the fixed GTK tooltip background whatever the
# wallpaper does.
C_TITLE='#61afef'
C_RULE='#5c6370'
C_LABEL='#abb2bf'
C_DIM='#5c6370'
C_GOOD='#98c379'
C_WARN='#e5c07b'
C_BAD='#e06c75'
C_EMPTY='#3e4451'

# Google Sans Flex leads the tooltip font stack and has proportional figures — "0"
# renders ~3px wider than "1" at 15px — so a column padded with %3s/%3d drifts by
# a few pixels per row and never lines up. The CSS stack ends in monospace for
# exactly this reason, but that tail is dead: Google Sans Flex covers all of ASCII
# itself, so it always wins and nothing ever reaches the fallback. Anything that
# has to line up column-for-column therefore names the family in the markup.
F_MONO='JetBrainsMono Nerd Font'

# Wrap the columnar part of a row — the meter and its number. The trailing prose
# (process names) is deliberately left out so it keeps the proportional face,
# which is both easier to read and narrower, buying the meter a few more cells.
mono() { printf '<span font_family="%s">%s</span>' "$F_MONO" "$1"; }

esc() { sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g'; }

# U+00A0, not a literal space: Pango's width request can drop trailing plain
# spaces, which would silently undo the right-margin fix below on whichever
# row happens to be last. NBSP survives that and renders identically to a
# space in every font this config uses.
NBSP=$(printf '\xc2\xa0')
IND2="$NBSP$NBSP"
IND3="$NBSP$NBSP$NBSP"

# Tooltip markup is written with double quotes; escaping them (and folding the
# newlines) happens once here, on the way into JSON.
#
# class is always emitted as a JSON array — waybar-custom(5): "The class
# parameter also accepts an array of strings." A single caller-passed class
# still works unchanged (a one-element array matches the same CSS), and a
# space-separated list (e.g. "portal eco") lets a caller layer a mode marker
# onto its normal state class without a second custom class of its own.
emit() { # class(es) space-separated, full bar markup, tooltip markup
	_em_cl=''
	for _em_c in $1; do _em_cl="$_em_cl${_em_cl:+,}\"$_em_c\""; done
	printf '{"text":"%s","class":[%s],"tooltip":"%s"}\n' \
		"$(printf '%s' "$2" | sed 's/"/\\"/g')" "$_em_cl" \
		"$(printf '%s' "$3" | sed 's/"/\\"/g' | sed ':a;N;$!ba;s/\n/\\n/g')"
}

# every bar module in this config wraps its icon identically
barico() { printf '<span size="115%%" rise="-1200">%s</span>' "$1"; }

title() { printf '<span font_weight="bold" foreground="%s">%s</span>\n' "$C_TITLE" "$1"; }
# Width in glyphs. N copies of one glyph is N times one advance, so this is a
# deterministic pixel width even in the proportional body font — no <tt> wrapper
# needed, and pinning a monospace family would be worse: fc-match monospace
# resolves to Noto Sans Mono and :charset=2500 to Verdana, so naming one would
# introduce a fallback that does not exist today.
#
# Pass a width to make the rule at least as wide as the widest row — it does
# not have to be the widest line any more (row()/dim()/sect() below carry their
# own matching margin on both sides now), but a rule shorter than its rows
# still looks wrong, so callers keep sizing it to their content.
rule() { # [cells=30]
	_ri=${1:-30} _rs=''
	while [ "$_ri" -gt 0 ]; do
		_rs="$_rs─"
		_ri=$((_ri - 1))
	done
	printf '<span foreground="%s">%s</span>\n' "$C_RULE" "$_rs"
}
# Left and right insets match — see NBSP above for why they are not plain
# spaces. The margin sits outside the coloured span, same as row()/dim(), so
# the icon-label gap stays a real space and only the outer margin is NBSP.
sect() { printf '\n%s<span foreground="%s">%s  %s</span>%s\n' "$IND2" "$C_LABEL" "$1" "$2" "$IND2"; }
row() { printf '%s%s%s\n' "$IND3" "$1" "$IND3"; }
dim() { printf '%s<span foreground="%s">%s</span>%s\n' "$IND3" "$C_DIM" "$1" "$IND3"; }
# Compact key/value row: an 8-cell label column in F_MONO (Google Sans Flex's
# proportional figures are exactly why row()'s %3s%% columns never lined up —
# a fixed-width label needs a fixed-width font, same reasoning as mono()
# below), then the value as-is. kvsub is a second, label-less row that lines
# up under the value column — an 8-space run in the same F_MONO span, so it
# measures identically to an 8-character label instead of drifting in the
# proportional face.
kv() { printf '%s<span font_family="%s">%-8s</span>%s%s\n' "$IND3" "$F_MONO" "$1" "$2" "$IND3"; }
kvsub() { printf '%s<span font_family="%s">%-8s</span><span foreground="%s">%s</span>%s\n' "$IND3" "$F_MONO" "" "$C_DIM" "$1" "$IND3"; }
good() { printf '<span foreground="%s">%s</span>' "$C_GOOD" "$1"; }
warn() { printf '<span foreground="%s">%s</span>' "$C_WARN" "$1"; }
bad() { printf '<span foreground="%s">%s</span>' "$C_BAD" "$1"; }

# pick the good/warn/bad colour for a percentage, given the two thresholds
grade() { # pct, warn-at, bad-at -> colour
	awk -v p="$1" -v w="$2" -v b="$3" -v g="$C_GOOD" -v y="$C_WARN" -v r="$C_BAD" \
		'BEGIN { print (p >= b ? r : p >= w ? y : g) }'
}

# 20-cell capsule, third argument narrows it for list rows. Colored NBSP runs
# on a bgcolor span rather than the old █/░ glyph run: a background rectangle
# is font-metric independent (no glyph to hunt for at a given size/weight, no
# gap between cells), and size="55%" is what turns the run into a thin bar
# with a visible track instead of a row of text — a redraw, not the same
# glyphs recolored, so anything measuring the old output has to be re-derived.
bar() { # pct, fill-color, [cells=20]
	awk -v p="$1" -v c="$2" -v w="${3:-20}" -v e="$C_EMPTY" -v nbsp="$NBSP" 'BEGIN {
		n = int(p * w / 100 + 0.5); if (n > w) n = w; if (n < 0) n = 0
		for (i = 0; i < n; i++)  f = f nbsp
		for (i = n; i < w; i++)  m = m nbsp
		printf "<span size=\"55%%\" background=\"%s\">%s</span><span size=\"55%%\" background=\"%s\">%s</span>", c, f, e, m
	}'
}

# One cell per value — for series too wide to give each a 20-cell bar (12 cores).
# Every cell takes its colour from its own value rather than one colour for the
# whole series: a flat-coloured row hides a single pinned core completely, which
# is the one thing a per-core row exists to show. `gap` inserts a space before
# that 1-based index, to part two groups (P-cores from E-cores).
#
# This replaced a single-colour `sparkbar`; nothing wanted the flat version once
# per-cell grading existed, and claudebar draws its own shapes in Rust.
heatbar() { # "p1 p2 ...", warn-at, bad-at, [gap-before-index]
	awk -v w="$2" -v b="$3" -v gap="${4:-0}" -v g="$C_GOOD" -v y="$C_WARN" -v r="$C_BAD" 'BEGIN {
		n = split(ARGV[1], p, " "); ARGC = 1
		split("▁ ▂ ▃ ▄ ▅ ▆ ▇ █", s, " ")
		for (i = 1; i <= n; i++) {
			if (i == gap) out = out " "
			k = int(p[i] / 12.5) + 1; if (k > 8) k = 8; if (k < 1) k = 1
			c = (p[i] >= b ? r : p[i] >= w ? y : g)
			out = out sprintf("<span foreground=\"%s\">%s</span>", c, s[k])
		}
		printf "%s", out
	}' "$1"
}

human() { # bytes/s -> "1.2 MB/s"
	awk -v b="$1" 'BEGIN {
		split("B kB MB GB", u, " ")
		i = 1; while (b >= 1024 && i < 4) { b /= 1024; i++ }
		printf (b >= 100 || i == 1 ? "%.0f %s/s" : "%.1f %s/s"), b, u[i]
	}'
}

hkib() { # KiB -> "3.4 GiB"
	awk -v k="$1" 'BEGIN {
		if (k == 0) { printf "none"; exit }
		split("KiB MiB GiB TiB", u, " ")
		i = 1; while (k >= 1024 && i < 4) { k /= 1024; i++ }
		printf (k >= 100 ? "%.0f %s" : "%.1f %s"), k, u[i]
	}'
}

hcount() { # 1234567 -> "1.2M"
	awk -v n="$1" 'BEGIN {
		split("|k|M|G", u, "|")
		i = 1; while (n >= 1000 && i < 4) { n /= 1000; i++ }
		printf (i == 1 ? "%d" : n >= 100 ? "%.0f%s" : "%.1f%s"), n, u[i]
	}'
}

hdur() { # seconds -> "2h 14m" / "14m" / "48s"
	awk -v s="$1" 'BEGIN {
		s = int(s)
		if (s >= 3600) printf "%dh %02dm", s / 3600, (s % 3600) / 60
		else if (s >= 60) printf "%dm %02ds", s / 60, s % 60
		else printf "%ds", s
	}'
}

# Cache a tooltip across polls faster than it needs to change — waybar re-execs
# a module on its text interval just to redraw one number, and the tooltip
# underneath is hidden ~99% of the time it is built. Key is caller-chosen: a
# bucketed timestamp for a TTL ("$(( $(date +%s) / 10 ))" = a new key every
# 10s) or content that only changes when the tooltip actually should
# ("$MINUTE|$POMODORO_STATE"). Cache file is "key\ntooltip-markup", and
# tip_stale reads only the first line, so a large tooltip costs nothing to
# check.
#
#   if tip_stale "$CACHE" "$KEY"; then TIP=$(…); tip_save "$CACHE" "$KEY" "$TIP"
#   else TIP=$(tip_load "$CACHE"); fi
tip_stale() { # cache-file, key
	{ IFS= read -r _ts_k; } < "$1" 2> /dev/null
	[ "${_ts_k:-}" = "$2" ] && return 1
	return 0
}
tip_load() { tail -n +2 "$1" 2> /dev/null; }
tip_save() { # cache-file, key, tooltip
	{ printf '%s\n' "$2"; printf '%s' "$3"; } > "$1.tmp" && mv -f "$1.tmp" "$1"
}

# mango/scripts/powermode.sh's mode file, read here rather than piped in so
# every module can call this without threading a new argument through. Missing
# file (mode never set, or a machine with no battery) reads as full — the
# safer default for a bar that has never been told otherwise.
power_mode() {
	_pm=full
	[ -r "${XDG_RUNTIME_DIR:-/tmp}/mango-powermode" ] && { IFS= read -r _pm < "${XDG_RUNTIME_DIR:-/tmp}/mango-powermode"; } 2> /dev/null
	printf '%s' "${_pm:-full}"
}

# Widen a tip_stale() TTL bucket in eco, collapse it to effectively no cache
# in full — "eco: refresh only when the state actually changes" vs.
# "full: refresh every poll" from the power-modes spec. full's bucket is 1s,
# not 0: every caller polls slower than that, so a new key every second reads
# as "always rebuild" without a division by zero.
tip_bucket() { # eco-ttl-seconds -> cache-key fragment
	_tb=1
	[ "$(power_mode)" = eco ] && _tb=${1:-60}
	printf '%s' "$(( $(date +%s) / _tb ))"
}

if [ "${1:-}" = "tooltip-selftest" ]; then
	# N copies of NBSP, for building the exact expected bar() string.
	_nbsp_n() { _i=$1 _o=''; while [ "$_i" -gt 0 ]; do _o="$_o$NBSP"; _i=$((_i - 1)); done; printf '%s' "$_o"; }
	_bar_exp() { printf '<span size="55%%" background="%s">%s</span><span size="55%%" background="%s">%s</span>' \
		"$C_GOOD" "$(_nbsp_n "$1")" "$C_EMPTY" "$(_nbsp_n "$2")"; }
	[ "$(bar 50 "$C_GOOD")" = "$(_bar_exp 10 10)" ] || { echo "bar 50 wrong"; exit 1; }
	[ "$(bar 0 "$C_GOOD")" = "$(_bar_exp 0 20)" ] || { echo "bar 0 wrong"; exit 1; }
	[ "$(bar 999 "$C_GOOD")" = "$(_bar_exp 20 0)" ] || { echo "bar clamp wrong"; exit 1; }
	[ "$(bar 50 "$C_GOOD" 10)" = "$(_bar_exp 5 5)" ] || { echo "narrow bar wrong"; exit 1; }
	# glyphs are no longer adjacent — each cell carries its own span
	heatbar "0 50 100" 70 90 | grep -q '▁</span>.*▅</span>.*█</span>' || { echo "heatbar glyph ramp wrong"; exit 1; }
	heatbar "0 75 95" 70 90 | grep -q "\"$C_GOOD\">▁</span><span foreground=\"$C_WARN\">▇</span><span foreground=\"$C_BAD\">█<" \
		|| { echo "heatbar should colour each cell by its own value"; exit 1; }
	heatbar "0 0 0" 70 90 3 | grep -q '</span> <span' || { echo "heatbar gap wrong"; exit 1; }
	heatbar "0 0 0" 70 90 | grep -q '</span> <span' && { echo "heatbar should not gap without an index"; exit 1; }
	# 3 bytes per ─, so 5 cells is 15 bytes. This is what pins tooltip width.
	[ "$(rule 5 | tr -cd '─' | wc -c)" -eq 15 ] || { echo "rule width wrong"; exit 1; }
	[ "$(rule | tr -cd '─' | wc -c)" -eq 90 ] || { echo "rule default width wrong"; exit 1; }
	# left and right margins must match — same NBSP run on both ends
	[ "$(dim x | grep -o "$NBSP*" | head -1)" = "$IND3" ] || { echo "dim left margin wrong"; exit 1; }
	[ "$(dim x | grep -o "$NBSP*\$")" = "$IND3" ] || { echo "dim right margin wrong"; exit 1; }
	[ "$(row x | grep -o "$NBSP*" | head -1)" = "$IND3" ] || { echo "row left margin wrong"; exit 1; }
	[ "$(row x | grep -o "$NBSP*\$")" = "$IND3" ] || { echo "row right margin wrong"; exit 1; }
	[ "$(sect i l | grep -o "$NBSP*\$")" = "$IND2" ] || { echo "sect right margin wrong"; exit 1; }
	# kv()'s label and kvsub()'s blank column must measure identically — both
	# an 8-cell run in F_MONO — or a continuation line drifts under the value.
	[ "$(kv Mode X)" = "${IND3}<span font_family=\"$F_MONO\">Mode    </span>X${IND3}" ] || { echo "kv wrong: $(kv Mode X)"; exit 1; }
	[ "$(kvsub Y)" = "${IND3}<span font_family=\"$F_MONO\">        </span><span foreground=\"$C_DIM\">Y</span>${IND3}" ] || { echo "kvsub wrong: $(kvsub Y)"; exit 1; }
	# emit()'s class is always a JSON array — a bare caller-passed class still
	# reads as one element, and a space-separated list becomes several.
	[ "$(emit ok x y | jq -r '.class | join(",")')" = ok ] || { echo "emit single class wrong"; exit 1; }
	[ "$(emit "ok eco" x y | jq -r '.class | join(",")')" = ok,eco ] || { echo "emit multi class wrong"; exit 1; }
	[ "$(grade 10 70 90)" = "$C_GOOD" ] && [ "$(grade 75 70 90)" = "$C_WARN" ] && [ "$(grade 95 70 90)" = "$C_BAD" ] || { echo "grade wrong"; exit 1; }
	[ "$(hkib 1048576)" = "1.0 GiB" ] || { echo "hkib wrong: $(hkib 1048576)"; exit 1; }
	[ "$(hkib 0)" = "none" ] || { echo "hkib 0 should read as none, not 0.0 KiB"; exit 1; }
	[ "$(hcount 1234567)" = "1.2M" ] || { echo "hcount wrong: $(hcount 1234567)"; exit 1; }
	[ "$(hdur 8040)" = "2h 14m" ] && [ "$(hdur 90)" = "1m 30s" ] && [ "$(hdur 9)" = "9s" ] || { echo "hdur wrong"; exit 1; }
	TC=$(mktemp)
	tip_stale "$TC" k1 || { echo "tip_stale should be stale for a missing/empty cache"; exit 1; }
	tip_save "$TC" k1 hello
	tip_stale "$TC" k1 && { echo "tip_stale should not be stale right after a save with the same key"; exit 1; }
	tip_stale "$TC" k2 || { echo "tip_stale should go stale once the key changes"; exit 1; }
	[ "$(tip_load "$TC")" = hello ] || { echo "tip_load wrong: $(tip_load "$TC")"; exit 1; }
	rm -f "$TC" "$TC.tmp"

	# power_mode reads a fixed runtime path, not an argument — fake it via XDG_RUNTIME_DIR
	XDG_RUNTIME_DIR=$(mktemp -d)
	[ "$(power_mode)" = full ] || { echo "power_mode with no state file should default to full"; exit 1; }
	printf 'eco' > "$XDG_RUNTIME_DIR/mango-powermode"
	[ "$(power_mode)" = eco ] || { echo "power_mode should read the state file"; exit 1; }
	B1=$(tip_bucket 3600)
	printf 'full' > "$XDG_RUNTIME_DIR/mango-powermode"
	B2=$(tip_bucket 3600)
	[ "$B1" != "$B2" ] || { echo "full mode should not share eco's wide bucket"; exit 1; }
	rm -rf "$XDG_RUNTIME_DIR"

	echo "ok"
	exit 0
fi
