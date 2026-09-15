#!/usr/bin/env bash
# Closes every inbound path to this host that VAULT.md flags open, except
# wg_hetzner, the mango hotspot (p2p0) and the docker bridges — those already
# have their own scoped ufw rules (system/remote/, system/libvirt-net/) or
# stay open on purpose (docker bridge -> host postgres/redis). ufw owns
# inbound; system/vpnguard/ owns egress — see README.md in this directory.
# Follows the system/libvirt-net/ precedent: sudo-gated, nothing under
# system/ is symlinked into ~/.config, so this script is the only way these
# rules land.
#
#   sudo src/system/firewall/install.sh
#   src/system/firewall/install.sh --print   # list every command; no root, changes nothing
#
# check: ufw:1234/tcp
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAEMON_JSON_SRC="$SRC_DIR/docker-daemon.json"
DAEMON_JSON_DEST="/etc/docker/daemon.json"
LAN_NET="172.16.0.0/12"
LMSTUDIO_PORT=1234

PRINT_ONLY=0
case "${1:-}" in
"") ;;
--print) PRINT_ONLY=1 ;;
*)
	echo "usage: $0 [--print]" >&2
	exit 2
	;;
esac

# say prints a progress/preview line in both modes. act runs the real command,
# only outside --print. Kept separate (not one line per step) because the
# text shown for "ufw enable" and for the /etc/default/ufw edit needs to read
# clearly even though the edit itself happens as a side effect of a ufw
# default command, not a direct file write.
say() { echo "==> $*"; }
act() { [ "$PRINT_ONLY" -eq 1 ] || "$@"; }

if [ "$PRINT_ONLY" -eq 0 ]; then
	[ "$(id -u)" -eq 0 ] || { echo "run me with sudo (or --print to preview)" >&2; exit 1; }
fi

command -v ufw >/dev/null 2>&1 || { echo "missing dependency: ufw" >&2; exit 1; }

if [ "$PRINT_ONLY" -eq 0 ]; then
	KVER="$(uname -r)"
	if [ ! -d "/lib/modules/$KVER" ]; then
		echo "no /lib/modules/$KVER on disk. A kernel upgrade removed the running" >&2
		echo "kernel's module directory without a reboot; 'ufw enable' then fails" >&2
		echo "loading netfilter modules (xt_LOG, xt_limit, the REJECT target) with" >&2
		echo "errors like 'RULE_APPEND failed'. See firewall.md Incident log." >&2
		echo "Reboot into the kernel that matches /lib/modules/, then rerun." >&2
		exit 1
	fi
fi

say "# /etc/default/ufw: DEFAULT_OUTPUT_POLICY=\"ACCEPT\" (set by the next command)"
say "ufw default deny incoming"
act ufw default deny incoming
say "ufw default allow outgoing"
act ufw default allow outgoing
say "ufw default allow routed"
act ufw default allow routed

say "ufw allow from $LAN_NET to any port $LMSTUDIO_PORT proto tcp comment 'lm studio proxy from docker'"
act ufw allow from "$LAN_NET" to any port "$LMSTUDIO_PORT" proto tcp comment 'lm studio proxy from docker'

say "install -D -m 0644 $DAEMON_JSON_SRC $DAEMON_JSON_DEST"
if [ "$PRINT_ONLY" -eq 0 ]; then
	if [ -e "$DAEMON_JSON_DEST" ] && ! cmp -s "$DAEMON_JSON_SRC" "$DAEMON_JSON_DEST"; then
		echo "$DAEMON_JSON_DEST already exists and differs from $DAEMON_JSON_SRC." >&2
		echo "Refusing to overwrite. Compare them by hand, then rerun." >&2
		exit 1
	fi
	install -D -m 0644 "$DAEMON_JSON_SRC" "$DAEMON_JSON_DEST"
fi

say "systemctl restart docker"
act systemctl restart docker

say "ufw enable"
act ufw enable

if [ "$PRINT_ONLY" -eq 0 ]; then
	say "verifying"
	STATUS="$(ufw status verbose)"
	for pat in '5900.*wg_hetzner' '5900.*p2p0' '5432,6379'; do
		printf '%s\n' "$STATUS" | grep -Eq "$pat" ||
			echo "WARNING: no ufw rule matching '$pat' in 'ufw status verbose' — check it by hand." >&2
	done
	if command -v ss >/dev/null 2>&1; then
		if ss -tlnpH 2>/dev/null | grep -E 'docker-proxy' | grep -Eq '0\.0\.0\.0|\[::\]'; then
			echo "WARNING: docker-proxy is still listening on 0.0.0.0 or [::]." >&2
			echo "A container publishes a port with no host IP and no daemon.json" >&2
			echo "default in effect yet — restart the container, or check that" >&2
			echo "$DAEMON_JSON_DEST loaded (docker info | grep -A1 'Bind Address')." >&2
		fi
	fi
	say "done"
fi

cat <<EOF

Rollback:
  sudo ufw delete allow from $LAN_NET to any port $LMSTUDIO_PORT proto tcp
  sudo rm -f $DAEMON_JSON_DEST   # only if this script created it — see README.md
  sudo systemctl restart docker
  sudo ufw disable
EOF
