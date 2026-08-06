#!/bin/sh
# Docker quick-actions menu for waybar's custom/docker (on-click).
# ponytail: one `docker` call per action — no daemon, no caching.
set -u

. "$(dirname "$0")/tooltip.sh"

DOCKER="${MANGO_DOCKER:-docker}"
FORMAT='{{.Names}}|{{.State}}|{{.Status}}|{{.Image}}|{{.Label "com.docker.compose.project"}}|{{.Label "com.docker.compose.service"}}'

DOT=$(printf '\xe2\x97\x8f')     # ● U+25CF — same decode-once reasoning as docker.sh
HOLLOW=$(printf '\xe2\x97\x8b')  # ○ U+25CB

notify() { notify-send -a waybar "Docker" "$1"; }

# custom/docker polls every 10s; nudging it (signal 15 — see config.jsonc's
# comment listing what else is taken) makes the pill catch up with whatever
# action just ran instead of waiting out the interval. On the trap so a
# cancelled menu is covered too.
trap 'pkill -RTMIN+15 waybar 2>/dev/null' EXIT INT TERM

# ---------------------------------------------------------------- primitives

# name|state|status|image|project|service on stdin -> running first, stable
# (order within each group is untouched) — running containers are what you
# reach for first from a click.
ordered() {
	awk -F'|' -v OFS='|' '{ key = ($2 == "running" ? 0 : 1); print key, $0 }' \
		| LC_ALL=C sort -t'|' -k1,1n -s | cut -d'|' -f2-
}

dot_class() { # state, status
	case "$2" in *'(unhealthy)'*) printf bad; return ;; esac
	case "$1" in restarting) printf warn; return ;; esac
	case "$1" in running) printf good; return ;; esac
	printf dim
}

dot() { # good|warn|bad|dim
	case "$1" in
	good) good "$DOT" ;;
	warn) warn "$DOT" ;;
	bad) bad "$DOT" ;;
	*) printf '<span foreground="%s">%s</span>' "$C_DIM" "$HOLLOW" ;;
	esac
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = "test" ]; then
	SAMPLE='a|exited|Exited (0) 2 days ago|img||svc-a
b|running|Up 1 minute|img||svc-b
c|restarting|Restarting (1) 5 seconds ago|img||svc-c'

	O=$(printf '%s\n' "$SAMPLE" | ordered)
	[ "$(printf '%s\n' "$O" | wc -l)" = 3 ] || { echo "ordered wrong count"; exit 1; }
	[ "$(printf '%s\n' "$O" | head -1 | cut -d'|' -f1)" = b ] || { echo "ordered should put running first"; exit 1; }

	[ "$(dot_class running 'Up 1 minute')" = good ] || { echo "dot_class running wrong"; exit 1; }
	[ "$(dot_class restarting 'Restarting (1) 5 seconds ago')" = warn ] || { echo "dot_class restarting wrong"; exit 1; }
	[ "$(dot_class exited 'Exited (0) 2 days ago')" = dim ] || { echo "dot_class exited wrong"; exit 1; }

	# IDX->action math, same scheme as wifi-menu.sh: IDX==N is the rule (no-op),
	# IDX==N+1/N+2 are the two action rows.
	N=3
	for pair in "3:0" "4:1" "5:2"; do
		idx=${pair%%:*}
		want=${pair##*:}
		got=$((idx - N))
		[ "$got" = "$want" ] || { echo "action math wrong for idx=$idx: got $got want $want"; exit 1; }
	done

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- menu

ROWS=$("$DOCKER" ps -a --format "$FORMAT" 2>&1)
if [ $? -ne 0 ]; then
	notify "Daemon unreachable: $ROWS"
	exit 0
fi
ROWS=$(printf '%s\n' "$ROWS" | grep .) || true  # drop the blank line when there are no containers at all

ORDERED=$(printf '%s\n' "$ROWS" | ordered)
N=0
[ -n "$ROWS" ] && N=$(printf '%s\n' "$ORDERED" | wc -l)

MENU=""
if [ "$N" -gt 0 ]; then
	MENU=$(printf '%s\n' "$ORDERED" | while IFS='|' read -r name state status image proj svc; do
		# full container name, not the compose service — "bench" alone can't
		# tell crema-v16's container from crema-develop's, unlike the tooltip
		# where the project section already disambiguates
		c=$(dot_class "$state" "$status")
		printf '%s  %s  <span foreground="%s">%s</span>\n' \
			"$(dot "$c")" "$(printf '%s' "$name" | esc)" "$C_DIM" "$(printf '%s' "$status" | esc)"
	done)
fi

RULE='<span alpha="30%">────────────────────</span>'
ACTIONS="  Stop all running
  Prune stopped"

if [ "$N" -gt 0 ]; then
	LIST=$(printf '%s\n%s\n%s' "$MENU" "$RULE" "$ACTIONS")
else
	LIST=$(printf '%s\n%s' "$RULE" "$ACTIONS")
fi

# Selection comes back as a row index, never the decorated pango label.
# kb-accept-alt has to be unbound before kb-custom-N can take Shift/Alt+Return.
IDX=$(printf '%s\n' "$LIST" |
	rofi -dmenu -format i -markup-rows -p "Docker" \
		-mesg "Shift+Enter restart  ·  Alt+Enter logs" \
		-theme-str 'window { width: 720px; } listview { spacing: 5px; } element { padding: 9px 8px; }' \
		-kb-accept-alt "" -kb-custom-1 "Shift+Return" -kb-custom-2 "Alt+Return")
RC=$?
[ "$RC" = 0 ] || [ "$RC" = 10 ] || [ "$RC" = 11 ] || exit 0
case "$IDX" in '' | *[!0-9]*) exit 0 ;; esac

if [ "$IDX" -ge "$N" ]; then
	case $((IDX - N)) in
	1) OUT=$("$DOCKER" stop $("$DOCKER" ps -q) 2>&1) || notify "Stop all failed: $OUT" ;;
	2) OUT=$("$DOCKER" container prune -f 2>&1) || notify "Prune failed: $OUT" ;;
	esac
	exit 0
fi

ROW=$(printf '%s\n' "$ORDERED" | sed -n "$((IDX + 1))p")
NAME=$(printf '%s\n' "$ROW" | cut -d'|' -f1)
STATE=$(printf '%s\n' "$ROW" | cut -d'|' -f2)

case "$RC" in
0)
	if [ "$STATE" = running ]; then
		OUT=$("$DOCKER" stop "$NAME" 2>&1) || notify "Stop failed: $OUT"
	else
		OUT=$("$DOCKER" start "$NAME" 2>&1) || notify "Start failed: $OUT"
	fi
	;;
10)
	OUT=$("$DOCKER" restart "$NAME" 2>&1) || notify "Restart failed: $OUT"
	;;
11)
	kitty --class mango-monitor -e "$DOCKER" logs -f --tail 200 "$NAME" &
	;;
esac
