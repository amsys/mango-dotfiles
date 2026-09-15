#!/usr/bin/env bash
# Memory and swap tuning: zram sizing, swap readahead, dirty-page caps, and
# turning zswap off so it stops shadowing zram. Follows the system/rapl/
# precedent: sudo-gated, and nothing under system/ is symlinked into
# ~/.config, so this script is the only way any of it lands.
#
#   sudo ~/src/mango-dotfiles/system/memtune/install.sh
#
# The reason this directory exists: Arch ships CONFIG_ZSWAP_DEFAULT_ON=y, so
# zswap runs in front of /dev/zram0 and compresses every page before zram
# sees it. zram then compresses the compressed data and gains nothing —
# measured 1.008:1 on 2026-09-08. Each payload file carries its own why.
#
# These files were live on nauthiz but tracked nowhere before this directory,
# 99-dirty.conf included — the QLC write-stall fix that has already taken the
# machine down once.

# check: /etc/tmpfiles.d/zswap.conf
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
[ -e /sys/module/zswap/parameters/enabled ] || { echo "no zswap in this kernel — nothing to disable, check the payload before installing" >&2; exit 1; }
command -v zramctl >/dev/null || { echo "zram-generator/zramctl is not installed" >&2; exit 1; }

echo "==> sysctl -> /etc/sysctl.d/99-dirty.conf"
install -D -m 0644 -o root -g root "$SRC_DIR/sysctl.d-99-dirty.conf" /etc/sysctl.d/99-dirty.conf

echo "==> sysctl -> /etc/sysctl.d/99-zram.conf"
install -D -m 0644 -o root -g root "$SRC_DIR/sysctl.d-99-zram.conf" /etc/sysctl.d/99-zram.conf

# 99-swappiness.conf set vm.swappiness=10 and was already dead: 99-zram.conf
# sorts after it and set 100. Leaving it in place is a trap — it reads as
# authoritative, and it takes effect again the moment either file is renamed.
if [ -e /etc/sysctl.d/99-swappiness.conf ]; then
	echo "==> removing dead /etc/sysctl.d/99-swappiness.conf (shadowed by 99-zram.conf)"
	cp -a /etc/sysctl.d/99-swappiness.conf "/etc/sysctl.d/99-swappiness.conf.bak.$STAMP"
	rm -f /etc/sysctl.d/99-swappiness.conf
fi

echo "==> tmpfiles -> /etc/tmpfiles.d/zswap.conf"
install -D -m 0644 -o root -g root "$SRC_DIR/tmpfiles.d-zswap.conf" /etc/tmpfiles.d/zswap.conf

echo "==> zram-generator -> /etc/systemd/zram-generator.conf"
[ -e /etc/systemd/zram-generator.conf ] &&
	cp -a /etc/systemd/zram-generator.conf "/etc/systemd/zram-generator.conf.bak.$STAMP"
install -D -m 0644 -o root -g root "$SRC_DIR/zram-generator.conf" /etc/systemd/zram-generator.conf

echo "==> applying sysctl and tmpfiles now"
sysctl --system >/dev/null
systemd-tmpfiles --create /etc/tmpfiles.d/zswap.conf

echo "==> verifying"
fail=0
got_zswap="$(cat /sys/module/zswap/parameters/enabled)"
got_swappiness="$(cat /proc/sys/vm/swappiness)"
got_cluster="$(cat /proc/sys/vm/page-cluster)"
echo "    zswap enabled   = $got_zswap   (want N)"
echo "    vm.swappiness   = $got_swappiness (want 100)"
echo "    vm.page-cluster = $got_cluster   (want 0)"
[ "$got_zswap" = "N" ]       || { echo "    zswap did not turn off" >&2; fail=1; }
[ "$got_swappiness" = "100" ] || { echo "    swappiness did not apply" >&2; fail=1; }
[ "$got_cluster" = "0" ]      || { echo "    page-cluster did not apply" >&2; fail=1; }
[ "$fail" -eq 0 ] || exit 1

cat <<EOF

Installed.

Two things are NOT live yet and need a reboot, at a moment you choose:
  - zram is still $(zramctl --noheadings --output DISKSIZE /dev/zram0 2>/dev/null | tr -d ' ' || echo '?'); the new size (ram / 2) applies when
    systemd-zram-setup@zram0 next runs.
  - zswap stops accepting new pages now, but the pages already in its pool
    stay until faulted in. 'grep Zswapped /proc/meminfo' reaches 0 after a
    reboot, not before.

After that reboot, the real check — run a cargo build, then:
  cat /sys/block/zram0/mm_stat   # cols 1,2 = original vs compressed bytes
The ratio was 1.008:1 with zswap in the way. It should now be well above
that, and it tells you what your workload actually compresses at.

Rollback:
  sudo rm /etc/tmpfiles.d/zswap.conf
  sudo cp -a /etc/systemd/zram-generator.conf.bak.$STAMP /etc/systemd/zram-generator.conf
  echo Y | sudo tee /sys/module/zswap/parameters/enabled
  # then reboot to restore the old zram size
EOF
