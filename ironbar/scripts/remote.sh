#!/bin/sh
# Remote-access toggle for ironbar's remote module: wayvnc (screen) +
# kdeconnectd (input/clipboard/files) + mango-keepawake (idle/lid block) +
# wayvnc-privacy (panel blanking), started/stopped together. Reachable only
# on wg_hetzner and the mango hotspot — see system/remote/install.sh for the
# ufw rules and system/remote/README.md for why. None of these units is
# enabled at login; this is the only way they run.
#
#   --toggle          on-click-left  — start/stop the whole group. Also
#                      creates a virtual (headless) output for wayvnc to
#                      capture, and destroys it again on stop. The remote
#                      user works on that output; the physical panels stay
#                      blanked by --privacy-watch. See --pull. Also
#                      regenerates and reloads the bar config, so the
#                      virtual output gets a bar of its own on start and
#                      loses it again on stop.
#   --pull M T         move tag T of physical monitor M onto the virtual
#                      output, so the remote user can see and use it. A tag
#                      pulled earlier goes back to its own monitor first —
#                      the virtual output holds one pulled tag at a time.
#   --pull-next        on-click-right / SUPER+CTRL+Next — pull the tag after
#                      the one currently pulled, cycling across both
#                      physical monitors' occupied tags. Reachable from a
#                      client that can only send modifiers + Tab/Esc/
#                      PgUp/PgDown/Home. Nothing pulled yet → pulls the
#                      first one.
#   --pull-prev        SUPER+CTRL+Prior — same, the other direction.
#   --pull-list        prints the candidate (monitor, tag) pairs as TSV,
#                      one per line, `*` marking the one currently pulled.
#                      Shared by --pull-next/--pull-prev and by hand-testing.
#   --restore          send every pulled client back to its own monitor
#                      and tag. Safe to run twice. Runs on VNC client
#                      disconnect (--privacy-watch) and on toggle-off.
#   --vnc-exec         wayvnc.service ExecStart — exec wayvnc, capturing
#                      the virtual output when one is recorded
#   --privacy-watch    wayvnc-privacy.service ExecStart — blanks every
#                      output for as long as a VNC client is connected
#   --privacy-restore  wayvnc-privacy.service ExecStopPost — undoes a
#                      blank left over by a crashed/killed watcher
#   --idle-wake        hypridle's 600s on-resume — skips turning the
#                       panels back on while privacy blanking is active
set -u

RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/mango-remote"
WLOPM_STATE="$RUNTIME_DIR/wlopm-state"
KEEPAWAKE_MARK="$RUNTIME_DIR/keepawake-ours"
# Name of the virtual output (HEADLESS-<n>). wlroots increments <n> on every
# create in one compositor run and never reuses a name, so the name must be
# discovered after each create, never hardcoded.
VOUT_FILE="$RUNTIME_DIR/virtual-output"
# One line per pulled client: "<id>\t<monitor>\t<tagmask>" (tagmask is
# mango's pipe-joined form, e.g. "4" or "4|5"), written by --pull and
# consumed by --restore.
PULLED="$RUNTIME_DIR/pulled-clients"
# "<monitor>\t<tag>\n" of the last --pull, regardless of whether it moved any
# client (an empty tag has none). Separate from PULLED: once pulled, a tag's
# clients report monitor==$VOUT in `all-clients`, so its origin disappears
# from the query --pull-next/--pull-list build their candidate list from —
# this is the one place that origin survives, for both to merge back in.
PULLED_ORIGIN="$RUNTIME_DIR/pulled-origin"

UNITS="wayvnc.service kdeconnectd.service wayvnc-privacy.service"

# Candidate (monitor, tag) pairs for --pull-next/--pull-prev/--pull-list:
# every tag occupied on a physical monitor, from `all-clients` — never
# `all-tags`: confirmed live that its `client_count` reports the whole
# compositor's client total on every occupied tag, not that tag's own count
# (three physical tags with 1/2/9 real clients each all read 20 there).
# The currently pulled origin is unioned in too, so the list's length
# doesn't change under the cycle just because a pull moved every client off
# a tag. One "<monitor>\t<tag>" per line, sorted and de-duplicated.
candidates() {
	vout="$1"
	origin="$2"
	{
		mmsg get all-clients 2>/dev/null |
			jq -r --arg v "$vout" \
				'.clients[] | select(.monitor != $v) | .monitor as $m | .tags[] | "\($m)\t\(.)"'
		[ -n "$origin" ] && printf '%s\n' "$origin"
	} | sort -u -t "$(printf '\t')" -k1,1 -k2,2n
}

case "${1:-}" in
--toggle)
	if systemctl --user is-active --quiet wayvnc.service; then
		if [ -e "$KEEPAWAKE_MARK" ]; then
			systemctl --user stop mango-keepawake.service
			rm -f "$KEEPAWAKE_MARK"
		fi
		"$0" --restore
		systemctl --user stop $UNITS || notify-send -a mango-bard "Remote access" "Failed to stop"
		mmsg dispatch destroy_all_virtual_output >/dev/null 2>&1
		rm -f "$VOUT_FILE"
	else
		mkdir -p "$RUNTIME_DIR" && chmod 0700 "$RUNTIME_DIR"
		# A crashed earlier toggle can leave a virtual output behind. Clear
		# all of them first so exactly one exists after create.
		mmsg dispatch destroy_all_virtual_output >/dev/null 2>&1
		mmsg dispatch create_virtual_output >/dev/null 2>&1
		VOUT=""
		for _ in 1 2 3 4 5 6 7 8 9 10; do
			VOUT=$(mmsg get all-monitors 2>/dev/null |
				jq -r '.monitors[].name | select(startswith("HEADLESS"))' | head -n1)
			[ -n "$VOUT" ] && break
			sleep 0.2
		done
		if [ -z "$VOUT" ]; then
			notify-send -a mango-bard "Remote access" "No virtual output — not starting"
			mango-bard refresh remote 2>/dev/null
			exit 1
		fi
		printf '%s\n' "$VOUT" >"$VOUT_FILE"
		if ! systemctl --user is-active --quiet mango-keepawake.service; then
			touch "$KEEPAWAKE_MARK"
			systemctl --user start mango-keepawake.service
		fi
		OUT=$(systemctl --user start $UNITS 2>&1) || notify-send -a mango-bard "Remote access" "Failed to start: $OUT"
	fi
	# Best-effort: the virtual output just appeared or disappeared, so the
	# generated bar config is stale either way. `ironbar reload` re-parses
	# it in the same process (no restart), which clears every ironvar it
	# was holding without this daemon's own cache noticing — "resync"
	# forces a full re-send past that stale-ACK cache (see main.rs's
	# dispatch_refresh). Chained with `&&`, not `;`: a failed gen-config
	# leaves the old config in place (write_atomic never partially writes),
	# so reloading or resyncing against it would be pointless.
	#
	# `ironbar reload` acks the IPC command before GTK finishes rebuilding
	# every bar (DP-1/eDP-1 included, not just the HEADLESS one) — a resync
	# fired straight after lands `@class/` sends mid-rebuild and gets
	# "Module not found" on the physical bars too, which briefly reset them
	# to their default styling (T-freeze-2026-08-29). ironbar has no signal
	# for "rebuild finished", so this fixed pause is a guess, not a fix —
	# `Vars::RETRY_COOLDOWN` (mango-bard/src/vars.rs) is the real backstop,
	# now short enough that a lost race here self-heals in well under a
	# second regardless. ponytail: revisit only if the pause itself proves
	# to miss the rebuild in practice.
	mango-bard gen-config 2>/dev/null && ironbar reload >/dev/null 2>&1 &&
		sleep 0.5 && mango-bard refresh resync -q 2>/dev/null
	mango-bard refresh remote 2>/dev/null
	;;
--vnc-exec)
	VOUT=$(cat "$VOUT_FILE" 2>/dev/null || true)
	if [ -n "$VOUT" ]; then
		exec /usr/bin/wayvnc -o "$VOUT" '[::]:5900'
	fi
	exec /usr/bin/wayvnc '[::]:5900'
	;;
--pull)
	MON="${2:-}"
	TAG="${3:-}"
	if [ -z "$MON" ] || [ -z "$TAG" ]; then
		echo "usage: remote.sh --pull <monitor> <tag>" >&2
		exit 1
	fi
	VOUT=$(cat "$VOUT_FILE" 2>/dev/null || true)
	if [ -z "$VOUT" ]; then
		notify-send -a mango-bard "Remote access" "VNC is off — nothing to pull to"
		exit 1
	fi
	"$0" --restore
	printf '%s\t%s\n' "$MON" "$TAG" >"$PULLED_ORIGIN"
	mmsg get all-clients 2>/dev/null |
		jq -r --arg m "$MON" --argjson t "$TAG" \
			'.clients[] | select(.monitor == $m and (.tags | index($t)))
			 | [(.id | tostring), .monitor, (.tags | map(tostring) | join("|"))] | @tsv' |
		while IFS="$(printf '\t')" read -r id mon tags; do
			printf '%s\t%s\t%s\n' "$id" "$mon" "$tags" >>"$PULLED"
			# tagmon by name, not tagcrossmon: tagcrossmon only retags in
			# place when the selected monitor already is the target, which
			# it is for every pull after the first (viewcrossmon below
			# selects the virtual output). The trailing 1 keeps the
			# client's own tags through the move.
			mmsg dispatch "tagmon,$VOUT,1" client,"$id" >/dev/null 2>&1
		done
	mmsg dispatch "viewcrossmon,$TAG,$VOUT" >/dev/null 2>&1
	mango-bard refresh remote 2>/dev/null
	;;
--restore)
	rm -f "$PULLED_ORIGIN"
	[ -e "$PULLED" ] || exit 0
	while IFS="$(printf '\t')" read -r id mon tags; do
		# The client can be gone by now; the dispatch then answers with an
		# error, which is the correct end state. tagmon keeps the client's
		# tags (see --pull for why not tagcrossmon).
		mmsg dispatch "tagmon,$mon,1" client,"$id" >/dev/null 2>&1
	done <"$PULLED"
	rm -f "$PULLED"
	mango-bard refresh remote 2>/dev/null
	;;
--pull-next | --pull-prev)
	VOUT=$(cat "$VOUT_FILE" 2>/dev/null || true)
	if [ -z "$VOUT" ]; then
		notify-send -a mango-bard "Remote access" "VNC is off"
		exit 0
	fi
	ORIGIN=$(cat "$PULLED_ORIGIN" 2>/dev/null || true)
	LIST=$(candidates "$VOUT" "$ORIGIN")
	if [ -z "$LIST" ]; then
		notify-send -a mango-bard "Remote access" "No tags to pull"
		exit 0
	fi
	N=$(printf '%s\n' "$LIST" | wc -l)
	FOUND=""
	[ -n "$ORIGIN" ] && FOUND=$(printf '%s\n' "$LIST" | grep -n -F -x "$ORIGIN" | cut -d: -f1)
	if [ -z "$FOUND" ]; then
		IDX=1
	elif [ "$1" = "--pull-next" ]; then
		IDX=$((FOUND % N + 1))
	else
		IDX=$(((FOUND - 2 + N) % N + 1))
	fi
	TARGET=$(printf '%s\n' "$LIST" | sed -n "${IDX}p")
	"$0" --pull "$(printf '%s' "$TARGET" | cut -f1)" "$(printf '%s' "$TARGET" | cut -f2)"
	;;
--pull-list)
	VOUT=$(cat "$VOUT_FILE" 2>/dev/null || true)
	ORIGIN=$(cat "$PULLED_ORIGIN" 2>/dev/null || true)
	candidates "$VOUT" "$ORIGIN" | while IFS= read -r line; do
		if [ "$line" = "$ORIGIN" ]; then
			printf '* %s\n' "$line"
		else
			printf '  %s\n' "$line"
		fi
	done
	;;
--privacy-watch)
	mkdir -p "$RUNTIME_DIR" && chmod 0700 "$RUNTIME_DIR"
	# Outer loop: at unit start, wayvncctl can attach to the previous
	# wayvnc's stale control socket, get EOF when the new wayvnc deletes
	# it, and exit 0 — which Restart=on-failure does not catch. Reconnect
	# for as long as wayvnc.service itself is active; systemd kills this
	# loop on unit stop.
	while systemctl --user is-active --quiet wayvnc.service; do
		wayvncctl -w -r event-receive 2>/dev/null | while IFS= read -r line; do
			case "$line" in
			*client-connected*)
				[ -e "$WLOPM_STATE" ] && continue
				wlopm -j >"$WLOPM_STATE.tmp" 2>/dev/null && mv "$WLOPM_STATE.tmp" "$WLOPM_STATE"
				wlopm --off '*'
				;;
			*client-disconnected*)
				[ -n "$(wayvncctl -j client-list 2>/dev/null | jq -c '.[]' 2>/dev/null)" ] && continue
				"$0" --restore
				"$0" --privacy-restore
				;;
			esac
		done
		sleep 1
	done
	;;
--privacy-restore)
	[ -e "$WLOPM_STATE" ] || exit 0
	jq -r '.[] | select(."power-mode" == "on") | .output' "$WLOPM_STATE" 2>/dev/null |
		while IFS= read -r output; do wlopm --on "$output"; done
	rm -f "$WLOPM_STATE"
	;;
--idle-wake)
	[ -e "$WLOPM_STATE" ] || wlopm --on '*'
	;;
*)
	echo "usage: remote.sh --toggle|--pull <mon> <tag>|--pull-next|--pull-prev|--pull-list|--restore|--vnc-exec|--privacy-watch|--privacy-restore|--idle-wake" >&2
	exit 1
	;;
esac
