#!/bin/sh
# Docker container indicator for waybar — running count in the bar, every
# container grouped by compose project in the tooltip.
#
# `docker stats` walks the cgroup tree per container and takes ~2s even with
# --no-stream, far too slow to poll every few seconds, so this tooltip has no
# per-container CPU/RAM column. `docker ps -a` is ~65ms and covers everything
# else — name, state, health, image, compose project — in one call.
#
#   docker.sh          custom/docker exec — emit JSON
#   docker.sh test     assert the parsers against canned `docker ps` output
set -u

. "$(dirname "$0")/tooltip.sh"

DOCKER="${MANGO_DOCKER:-docker}"
FORMAT='{{.Names}}|{{.State}}|{{.Status}}|{{.Image}}|{{.Label "com.docker.compose.project"}}|{{.Label "com.docker.compose.service"}}'

# printf only expands \xHH escapes when they sit in its own format string, not
# when they arrive via %s from a variable — so these are decoded once, here,
# rather than passed around as escape text (which would leak literal
# backslashes into the tooltip and break emit()'s JSON).
DOT=$(printf '\xe2\x97\x8f')     # ● U+25CF
HOLLOW=$(printf '\xe2\x97\x8b')  # ○ U+25CB

ic_docker() { printf '\xef\x8c\x88'; }  # nf-linux-docker (whale)  U+F308

# ---------------------------------------------------------------- primitives

# name|state|status|image|project|service, one container per line, on stdin.
# Grouped by project (stable sort — order within a project is untouched) with
# unlabeled containers pushed after every real project via "~", which sorts
# after any lowercase/digit/hyphen project name in the C locale, and rendered
# under "standalone".
grouped() {
	awk -F'|' -v OFS='|' '{ key = ($5 == "" ? "~" : $5); print key, $0 }' \
		| LC_ALL=C sort -t'|' -k1,1 -s
}

# good/warn/bad/dim, in that priority: an unhealthy container outranks a
# restarting one, health-starting outranks plain running.
dot_class() { # state, status
	case "$2" in *'(unhealthy)'*) printf bad; return ;; esac
	case "$1" in restarting) printf warn; return ;; esac
	case "$2" in *'(health: starting)'*) printf warn; return ;; esac
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

health_mark() { # status -> " ✓" or ""
	case "$1" in *'(healthy)'*) printf ' \xe2\x9c\x93' ;; esac
}

# Unlabeled project header line, deliberately icon-less: sect() always takes
# an icon and every icon on this bar already means something specific — a
# project name doesn't need one to read as a header.
projhdr() { printf '\n<span foreground="%s">  %s</span>\n' "$C_LABEL" "$1"; }

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = "test" ]; then
	SAMPLE='crema-v16-mariadb-1|running|Up 3 minutes (healthy)|mariadb:11.8|crema-v16|mariadb
crema-v16-redis-cache-1|running|Up 3 minutes|redis:alpine|crema-v16|redis-cache
crema-develop-mariadb-1|running|Up 4 minutes (health: starting)|mariadb:11.8|crema-develop|mariadb
old-postgres-1|exited|Exited (0) 2 days ago|postgres:15||
loose-1|restarting|Restarting (1) 5 seconds ago|busybox||'

	G=$(printf '%s\n' "$SAMPLE" | grouped)
	[ "$(printf '%s\n' "$G" | wc -l)" = 5 ] || { echo "grouped wrong count"; exit 1; }
	printf '%s\n' "$G" | tail -2 | grep -q '^~|old-postgres-1' || { echo "grouped should push unlabeled containers last"; exit 1; }
	printf '%s\n' "$G" | head -2 | grep -q '^crema-develop|' || { echo "grouped should sort projects"; exit 1; }

	[ "$(dot_class running 'Up 3 minutes (healthy)')" = good ] || { echo "dot_class healthy wrong"; exit 1; }
	[ "$(dot_class running 'Up 3 minutes (unhealthy)')" = bad ] || { echo "dot_class unhealthy wrong"; exit 1; }
	[ "$(dot_class running 'Up 4 minutes (health: starting)')" = warn ] || { echo "dot_class starting wrong"; exit 1; }
	[ "$(dot_class restarting 'Restarting (1) 5 seconds ago')" = warn ] || { echo "dot_class restarting wrong"; exit 1; }
	[ "$(dot_class exited 'Exited (0) 2 days ago')" = dim ] || { echo "dot_class exited wrong"; exit 1; }

	[ -n "$(health_mark 'Up 3m (healthy)')" ] || { echo "health_mark should mark healthy"; exit 1; }
	[ -z "$(health_mark 'Up 3m')" ] || { echo "health_mark should stay silent with no healthcheck"; exit 1; }

	RUNNING=$(printf '%s\n' "$SAMPLE" | awk -F'|' '$2 == "running"' | wc -l)
	[ "$RUNNING" = 3 ] || { echo "running count wrong: $RUNNING"; exit 1; }

	# module-level warning: a restarting or unhealthy row must flip it, a plain
	# exited/starting one must not
	printf '%s\n' "$SAMPLE" | awk -F'|' '$2 == "restarting" { f = 1 } $3 ~ /\(unhealthy\)/ { f = 1 } END { exit f ? 1 : 0 }'
	[ $? -ne 0 ] || { echo "class check should have flagged the restarting row"; exit 1; }
	printf 'a|running|Up 1 minute (healthy)|x|p|s\n' | awk -F'|' '$2 == "restarting" { f = 1 } $3 ~ /\(unhealthy\)/ { f = 1 } END { exit f ? 1 : 0 }'
	[ $? -eq 0 ] || { echo "class check should stay quiet with nothing wrong"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- module

ROWS=$("$DOCKER" ps -a --format "$FORMAT" 2> /dev/null)
if [ $? -ne 0 ]; then
	# daemon unreachable — empty text hides a custom module in waybar, same as
	# net.sh does while a scan is in flight; no error pill, no tofu.
	emit normal "" ""
	exit 0
fi

RUNNING=$(printf '%s\n' "$ROWS" | awk -F'|' '$2 == "running"' | wc -l)
STOPPED=$(printf '%s\n' "$ROWS" | awk -F'|' '$2 != "running" && $2 != ""' | wc -l)
PROJECTS=$(printf '%s\n' "$ROWS" | awk -F'|' '$5 != "" { p[$5] = 1 } END { print length(p) }')

CLASS=normal
printf '%s\n' "$ROWS" | awk -F'|' '$2 == "restarting" { f = 1 } $3 ~ /\(unhealthy\)/ { f = 1 } END { exit f ? 1 : 0 }' \
	|| CLASS=warning

TEXT="$(barico "$(ic_docker)") ${RUNNING}"

# `docker ps -a` itself is cheap (~18ms); the per-container row loop below is
# the real cost, and is invisible ~99% of the time — cache it on a slower
# clock (30s) than the 10s poll. Keyed on RUNNING+STOPPED so a container
# starting or stopping invalidates it immediately rather than waiting out the
# TTL.
TIP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-docker-tip"
TIP_KEY="$RUNNING-$STOPPED-$(( $(date +%s) / 30 ))"
if tip_stale "$TIP_CACHE" "$TIP_KEY"; then
TIP=$(
	title "Docker"
	rule 44

	PREV=""
	printf '%s\n' "$ROWS" | grouped | while IFS='|' read -r key name state status image proj svc; do
		[ -n "$name" ] || continue
		if [ "$proj" != "$PREV" ]; then
			projhdr "$(printf '%s' "${proj:-standalone}" | esc)"
			PREV="$proj"
		fi
		label="${svc:-$name}"
		c=$(dot_class "$state" "$status")
		row "$(printf '%s  %s <span foreground="%s">%s</span>' \
			"$(mono "$(printf '%s %-24s' "$(dot "$c")" "$(printf '%s' "$status$(health_mark "$status")" | esc)")")" \
			"$(printf '%s' "$label" | esc)" "$C_DIM" "$(printf '%s' "$image" | esc)")"
	done

	printf '\n'
	dim "$(printf '%s stopped  \xc2\xb7  %s project%s' "$STOPPED" "$PROJECTS" "$([ "$PROJECTS" = 1 ] || printf s)")"
)
	tip_save "$TIP_CACHE" "$TIP_KEY" "$TIP"
else
	TIP=$(tip_load "$TIP_CACHE")
fi

emit "$CLASS" "$TEXT" "$TIP"
