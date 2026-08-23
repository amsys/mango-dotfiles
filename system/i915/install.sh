#!/usr/bin/env bash
# Raise i915's render-engine preempt_timeout_ms so a long Vulkan compute
# batch (Ollama, llama.cpp) doesn't get declared a GPU hang while the
# compositor is sharing the same engine. Follows the system/rapl/ precedent:
# sudo-gated, and nothing under system/ is symlinked into ~/.config, so this
# script is the only way it lands.
#
#   sudo ~/src/mango-dotfiles/system/i915/install.sh
#
# See 99-i915-compute-timeouts.rules for why, and
# plans/iterative-watching-spring.md for the crash this addresses.

set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
[ -d /sys/module/i915 ] || { echo "i915 is not loaded" >&2; exit 1; }

echo "==> udev rule -> /etc/udev/rules.d/99-i915-compute-timeouts.rules"
install -m 0644 -o root -g root "$SRC_DIR/99-i915-compute-timeouts.rules" \
	/etc/udev/rules.d/99-i915-compute-timeouts.rules

echo "==> reloading udev rules"
udevadm control --reload-rules

echo "==> applying to already-present card(s)"
# ACTION=="add" only fires when the device is (re)created, so cards already
# present before this rule existed need one explicit trigger. Every reboot
# after this needs no such nudge.
for card in /sys/class/drm/card[0-9]*; do
	[ -e "$card/device/driver" ] || continue
	[ "$(basename "$(readlink -f "$card/device/driver")")" = "i915" ] || continue
	udevadm trigger --action=add "$card"
done

# The trigger above queues udev events on a worker and returns immediately —
# reading the sysfs value right after it can race the write and see the old
# value. settle blocks until the queue drains.
udevadm settle

echo "==> restoring heartbeat_interval_ms to its stock default (2500), if raised"
# An earlier version of this rule also raised heartbeat_interval_ms. That
# combination — heartbeat_interval_ms changed while ending up under 2x
# preempt_timeout_ms — makes i915 downgrade individual engine resets to full
# GPU resets (drm/i915/gt/intel_engine_heartbeat.c), which is a strictly
# worse version of the exact failure this rule exists to reduce. This rule no
# longer touches heartbeat_interval_ms at all, so repair any machine that hit
# the earlier version.
for card in /sys/class/drm/card[0-9]*/engine/rcs0/heartbeat_interval_ms; do
	[ -w "$card" ] || continue
	[ "$(cat "$card" 2>/dev/null)" = "2500" ] && continue
	echo "    $card"
	echo 2500 > "$card"
done

echo "==> verifying"
for card in /sys/class/drm/card[0-9]*/engine/rcs0; do
	[ -d "$card" ] || continue
	got_preempt="$(cat "$card/preempt_timeout_ms" 2>/dev/null || echo '?')"
	echo "    $card: preempt_timeout_ms=$got_preempt"
	if [ "$got_preempt" != "20000" ]; then
		echo "    did not apply — rule syntax or driver match may be off, check with:" >&2
		echo "      udevadm test -a add $(dirname "$(dirname "$card")")" >&2
		exit 1
	fi
done

cat <<EOF

Installed.

Rollback:
  sudo rm /etc/udev/rules.d/99-i915-compute-timeouts.rules
  sudo udevadm control --reload-rules
  # values stay raised until the next boot or i915 module reload; harmless either way
EOF
