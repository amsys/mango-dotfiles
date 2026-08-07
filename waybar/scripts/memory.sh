#!/bin/sh
# Memory indicator for waybar — RAM, swap and page faults in one tooltip.
#
# Swap used to be its own bar module. It never moves on this machine (a cold
# 4 GiB swapfile, priority -1), so a permanent "0%" pill was pure noise; it now
# lives in this tooltip and only colours the bar icon if something actually
# swaps out.
#
# DIMM identification needs root (/sys/firmware/dmi is 0400), so install-config.sh
# caches `dmidecode -t memory` once and this reads the cache. Missing cache just
# drops the Hardware section.
#
#   memory.sh          custom/memory exec — emit JSON
#   memory.sh test     assert the parsers against canned /proc text
set -u

. "$(dirname "$0")/tooltip.sh"

STATE="${XDG_RUNTIME_DIR:-/tmp}/waybar-memory"
DMI_CACHE="${MANGO_DMI_CACHE:-$HOME/.cache/mango-meminfo}"

ic_mem() { printf '\xef\x9e\xa3'; }  # memory_alt  U+F7A3

IC_RAM='󰍛'    # md-memory                U+F035B
IC_SWAP='󰓡'   # md-swap_horizontal       U+F04E1
IC_FAULT='󰗖'  # md-alert_circle_outline  U+F05D6
IC_TOP='󰉹'    # md-format_list_bulleted  U+F0279
IC_HW='󰘚'     # md-chip                  U+F061A

# ---------------------------------------------------------------- primitives

# Every /proc/meminfo key as a shell-safe "key=value" (values in KiB).
mem_kv() { awk '{ gsub(/[:()]/, "", $1); print $1 "=" $2 }' "${1:-/proc/meminfo}"; }

# pgfault / pgmajfault totals
vm_kv() { awk '$1 == "pgfault" || $1 == "pgmajfault" { print $1 "=" $2 }' "${1:-/proc/vmstat}"; }

# per-second rate between two counter samples
rate() { # now, prev, seconds
	awk -v n="$1" -v p="$2" -v s="$3" 'BEGIN {
		d = n - p
		if (s <= 0 || d < 0) { print 0; exit }
		printf "%d", d / s + 0.5
	}'
}

# "/swapfile file 4194300 0 -1" -> "swapfile · file"
swap_desc() { # /proc/swaps text
	awk 'NR > 1 { n = split($1, p, "/"); printf "%s · %s\n", p[n], $2; exit }'
}

# Populated DIMMs out of the dmidecode cache: "size|type|speed|part".
dimms() {
	awk '
		/^Memory Device$/ { s = ""; t = ""; sp = ""; pn = ""; loc = ""; have = 1; next }
		!have { next }
		/^\t*Size:/ { sub(/^[^:]*: */, ""); s = $0 }
		/^\t*Type:/ && !/Type Detail/ { sub(/^[^:]*: */, ""); t = $0 }
		/^\t*Configured Memory Speed:/ { sub(/^[^:]*: */, ""); sp = $0 }
		/^\t*Part Number:/ { sub(/^[^:]*: */, ""); pn = $0
			if (s !~ /No Module/ && s != "") printf "%s|%s|%s|%s\n", s, t, sp, pn
			have = 0 }
	' "${1:-$DMI_CACHE}" 2>/dev/null
}

# ---------------------------------------------------------------- self-check

if [ "${1:-}" = "test" ]; then
	T=$(mktemp -d)
	trap 'rm -rf "$T"' EXIT

	cat > "$T/meminfo" <<-'EOF'
		MemTotal:       48932736 kB
		MemFree:         2000000 kB
		MemAvailable:   30000000 kB
		Buffers:          500000 kB
		Cached:         20000000 kB
		Shmem:           1000000 kB
		Dirty:              4096 kB
		Writeback:             0 kB
		SwapTotal:       4194300 kB
		SwapFree:        4194300 kB
	EOF
	eval "$(mem_kv "$T/meminfo")"
	[ "$MemTotal" = 48932736 ] && [ "$MemAvailable" = 30000000 ] || { echo "mem_kv wrong"; exit 1; }
	[ "$(hkib $((MemTotal - MemAvailable)))" = "18.1 GiB" ] || { echo "used wrong: $(hkib $((MemTotal - MemAvailable)))"; exit 1; }

	printf 'pgfault 1000\npgmajfault 40\nnr_dirty 3\n' > "$T/vmstat"
	eval "$(vm_kv "$T/vmstat")"
	[ "$pgfault" = 1000 ] && [ "$pgmajfault" = 40 ] || { echo "vm_kv wrong"; exit 1; }
	[ "$(rate 1000 400 2)" = "300" ] || { echo "rate wrong: $(rate 1000 400 2)"; exit 1; }
	[ "$(rate 10 1000 2)" = "0" ] || { echo "counter reset must clamp"; exit 1; }
	[ "$(rate 10 1 0)" = "0" ] || { echo "zero elapsed must clamp"; exit 1; }

	printf 'Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n/swapfile file 4194300 0 -1\n' \
		| swap_desc | grep -qx 'swapfile · file' || { echo "swap_desc wrong"; exit 1; }

	cat > "$T/dmi" <<-'EOF'
		Memory Device
			Size: No Module Installed
			Type: Unknown
			Configured Memory Speed: Unknown
			Part Number: Not Specified
		Memory Device
			Size: 32 GB
			Type: DDR5
			Type Detail: Synchronous
			Configured Memory Speed: 5600 MT/s
			Part Number: CT32G56C46S5
	EOF
	[ "$(dimms "$T/dmi")" = "32 GB|DDR5|5600 MT/s|CT32G56C46S5" ] || { echo "dimms wrong: $(dimms "$T/dmi")"; exit 1; }

	echo "ok"
	exit 0
fi

# ---------------------------------------------------------------- module

eval "$(mem_kv)"
eval "$(vm_kv)"

USED=$((MemTotal - MemAvailable))
PCT=$(awk -v u="$USED" -v t="$MemTotal" 'BEGIN { printf "%d", u * 100 / t + 0.5 }')

SWAPUSED=$((SwapTotal - SwapFree))
SWAPPCT=0
[ "$SwapTotal" -gt 0 ] && SWAPPCT=$(awk -v u="$SWAPUSED" -v t="$SwapTotal" 'BEGIN { printf "%d", u * 100 / t + 0.5 }')

CLASS=normal
[ "$SWAPPCT" -gt 0 ] && CLASS=swapping
[ "$PCT" -ge 95 ] && CLASS=warning

TEXT="$(barico "$(ic_mem)") ${PCT}%"

# Invisible ~99% of the time. Full mode rebuilds every poll, eco rebuilds at
# most every 60s — see tip_bucket() in tooltip.sh. Ceiling in eco: "Top by
# RSS" and the fault rates can be up to 60s stale.
TIP_CACHE="${XDG_RUNTIME_DIR:-/tmp}/waybar-memory-tip"
TIP_KEY=$(tip_bucket 60)
if tip_stale "$TIP_CACHE" "$TIP_KEY"; then

# Fault rates need an interval, so keep the previous sample next to its clock.
NOW=$(awk '{ printf "%d", $1 }' /proc/uptime)
MINRATE=0 MAJRATE=0
if [ -r "$STATE" ]; then
	# shellcheck disable=SC2046  # deliberate word split of the three saved fields
	set -- $(cat "$STATE")
	MINRATE=$(rate "$pgfault" "$1" $((NOW - $3)))
	MAJRATE=$(rate "$pgmajfault" "$2" $((NOW - $3)))
fi
printf '%s %s %s\n' "$pgfault" "$pgmajfault" "$NOW" > "$STATE"

TIP=$(
	title "Memory"
	# Fixed width, same 44 cells as cpu.sh so the pair reads as one system —
	# see the rule() comment in tooltip.sh for why this pins the width.
	rule 44

	sect "$IC_RAM" "RAM"
	row "$(printf '%3s%%  %s' "$PCT" "$(bar "$PCT" "$(grade "$PCT" 75 90)")")"
	row "$(hkib "$USED") used of $(hkib "$MemTotal")  ·  $(hkib "$MemAvailable") available"
	dim "$(hkib "$Cached") cached  ·  $(hkib "$Buffers") buffers  ·  $(hkib "$Shmem") shared"
	dim "$(hkib "$Dirty") dirty  ·  $(hkib "$Writeback") in writeback"

	sect "$IC_SWAP" "Swap"
	if [ "$SwapTotal" -gt 0 ]; then
		row "$(printf '%3s%%  %s' "$SWAPPCT" "$(bar "$SWAPPCT" "$(grade "$SWAPPCT" 20 50)")")"
		row "$(hkib "$SWAPUSED") used of $(hkib "$SwapTotal")"
		dim "$(swap_desc < /proc/swaps)"
	else
		dim "none configured"
	fi

	# Major faults are the ones that hurt: they went to disk. Minor faults are
	# just the kernel handing out pages and run in the hundreds of thousands.
	sect "$IC_FAULT" "Page faults"
	row "$(printf '%s/s minor  ·  %s' "$(hcount "$MINRATE")" \
		"$([ "$MAJRATE" -gt 0 ] && bad "$MAJRATE/s major" || good "no major faults")")"
	dim "$(hcount "$pgfault") minor, $(hcount "$pgmajfault") major since boot"

	# Bars are relative to the biggest process, not to total RAM: on 48 GiB even
	# a 1.3 GiB hog is 3% and every bar would render empty.
	sect "$IC_TOP" "Top by RSS"
	TOP=$(ps -eo rss=,pid=,comm= --sort=-rss 2> /dev/null | head -5)
	TOPRSS=$(printf '%s\n' "$TOP" | awk 'NR == 1 { print $1 }')
	printf '%s\n' "$TOP" | while read -r rss pid c; do
		rpct=$(awk -v r="$rss" -v t="${TOPRSS:-1}" 'BEGIN { printf "%d", (t > 0 ? r * 100 / t + 0.5 : 0) }')
		# length is relative to the biggest process, colour is the real share of RAM
		share=$(awk -v r="$rss" -v t="$MemTotal" 'BEGIN { printf "%d", r * 100 / t + 0.5 }')
		row "$(printf '%s  %s <span foreground="%s">%s</span>' \
			"$(mono "$(printf '%s %9s' "$(bar "$rpct" "$(grade "$share" 10 25)" 14)" "$(hkib "$rss")")")" \
			"$(printf '%s' "$c" | esc)" "$C_DIM" "$pid")"
	done

	HW=$(dimms)
	if [ -n "$HW" ]; then
		sect "$IC_HW" "Hardware"
		printf '%s\n' "$HW" | while IFS='|' read -r sz ty sp pn; do
			row "$(printf '%s %s  ·  %s' "$sz" "$ty" "$sp" | esc)"
			dim "$(printf '%s' "$pn" | esc)"
		done
	fi
)
	tip_save "$TIP_CACHE" "$TIP_KEY" "$TIP"
else
	TIP=$(tip_load "$TIP_CACHE")
fi

emit "$CLASS" "$TEXT" "$TIP"
