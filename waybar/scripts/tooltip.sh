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

# Tooltip markup is written with double quotes; escaping them (and folding the
# newlines) happens once here, on the way into JSON.
emit() { # class, full bar markup, tooltip markup
	printf '{"text":"%s","class":"%s","tooltip":"%s"}\n' \
		"$(printf '%s' "$2" | sed 's/"/\\"/g')" "$1" \
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
# Pass a width to make the rule the widest line in the tooltip. That is what
# pins the tooltip to a constant width, and it is also what makes the left and
# right margins match: row()/dim() inset 3 spaces on the left and nothing on the
# right, so whenever a *row* is the widest line it sits flush against the right
# padding while everything else is indented.
rule() { # [cells=30]
	_ri=${1:-30} _rs=''
	while [ "$_ri" -gt 0 ]; do
		_rs="$_rs─"
		_ri=$((_ri - 1))
	done
	printf '<span foreground="%s">%s</span>\n' "$C_RULE" "$_rs"
}
sect() { printf '\n<span foreground="%s">  %s  %s</span>\n' "$C_LABEL" "$1" "$2"; }
row() { printf '   %s\n' "$1"; }
dim() { printf '   <span foreground="%s">%s</span>\n' "$C_DIM" "$1"; }
good() { printf '<span foreground="%s">%s</span>' "$C_GOOD" "$1"; }
warn() { printf '<span foreground="%s">%s</span>' "$C_WARN" "$1"; }
bad() { printf '<span foreground="%s">%s</span>' "$C_BAD" "$1"; }

# pick the good/warn/bad colour for a percentage, given the two thresholds
grade() { # pct, warn-at, bad-at -> colour
	awk -v p="$1" -v w="$2" -v b="$3" -v g="$C_GOOD" -v y="$C_WARN" -v r="$C_BAD" \
		'BEGIN { print (p >= b ? r : p >= w ? y : g) }'
}

# 20-cell bar, claudebar's ██░░ style. Third argument narrows it for list rows.
bar() { # pct, fill-color, [cells=20]
	awk -v p="$1" -v c="$2" -v w="${3:-20}" -v e="$C_EMPTY" 'BEGIN {
		n = int(p * w / 100 + 0.5); if (n > w) n = w; if (n < 0) n = 0
		for (i = 0; i < n; i++)  f = f "█"
		for (i = n; i < w; i++)  m = m "░"
		printf "<span foreground=\"%s\">%s</span><span foreground=\"%s\">%s</span>", c, f, e, m
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

if [ "${1:-}" = "tooltip-selftest" ]; then
	bar 50 "$C_GOOD" | grep -q '>██████████</span><span foreground="#3e4451">░░░░░░░░░░<' || { echo "bar 50 wrong"; exit 1; }
	bar 0 "$C_GOOD" | grep -q '></span><span foreground="#3e4451">░░░░░░░░░░░░░░░░░░░░<' || { echo "bar 0 wrong"; exit 1; }
	bar 999 "$C_GOOD" | grep -q '>████████████████████</span><span foreground="#3e4451"><' || { echo "bar clamp wrong"; exit 1; }
	bar 50 "$C_GOOD" 10 | grep -q '>█████</span><span foreground="#3e4451">░░░░░<' || { echo "narrow bar wrong"; exit 1; }
	# glyphs are no longer adjacent — each cell carries its own span
	heatbar "0 50 100" 70 90 | grep -q '▁</span>.*▅</span>.*█</span>' || { echo "heatbar glyph ramp wrong"; exit 1; }
	heatbar "0 75 95" 70 90 | grep -q "\"$C_GOOD\">▁</span><span foreground=\"$C_WARN\">▇</span><span foreground=\"$C_BAD\">█<" \
		|| { echo "heatbar should colour each cell by its own value"; exit 1; }
	heatbar "0 0 0" 70 90 3 | grep -q '</span> <span' || { echo "heatbar gap wrong"; exit 1; }
	heatbar "0 0 0" 70 90 | grep -q '</span> <span' && { echo "heatbar should not gap without an index"; exit 1; }
	# 3 bytes per ─, so 5 cells is 15 bytes. This is what pins tooltip width.
	[ "$(rule 5 | tr -cd '─' | wc -c)" -eq 15 ] || { echo "rule width wrong"; exit 1; }
	[ "$(rule | tr -cd '─' | wc -c)" -eq 90 ] || { echo "rule default width wrong"; exit 1; }
	[ "$(grade 10 70 90)" = "$C_GOOD" ] && [ "$(grade 75 70 90)" = "$C_WARN" ] && [ "$(grade 95 70 90)" = "$C_BAD" ] || { echo "grade wrong"; exit 1; }
	[ "$(hkib 1048576)" = "1.0 GiB" ] || { echo "hkib wrong: $(hkib 1048576)"; exit 1; }
	[ "$(hkib 0)" = "none" ] || { echo "hkib 0 should read as none, not 0.0 KiB"; exit 1; }
	[ "$(hcount 1234567)" = "1.2M" ] || { echo "hcount wrong: $(hcount 1234567)"; exit 1; }
	[ "$(hdur 8040)" = "2h 14m" ] && [ "$(hdur 90)" = "1m 30s" ] && [ "$(hdur 9)" = "9s" ] || { echo "hdur wrong"; exit 1; }
	echo "ok"
	exit 0
fi
