#!/usr/bin/env bash
# Install the mango-sddm login screen and the tool that keeps it in step with
# matugen. Idempotent — re-run it after any change to the theme, the colour
# template, or sddm-theme-sync.
#
#   sudo ~/src/mango-dotfiles/system/sddm/install.sh
#
# Ordered so that /etc/sddm.conf — the only step that changes what actually
# boots — goes last. Until that line runs, the previous login screen is still
# what you get, and everything before it is safe to abandon.

# check: /usr/share/sddm/themes/mango-sddm
# risk: boot
set -eu

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SRC_DIR/../.." && pwd)"
THEME_DIR=/usr/share/sddm/themes/mango-sddm
TEMPLATE="$REPO_DIR/matugen/templates/sddm/Colors.qml"
REFERENCE=/usr/local/share/sddm-theme-sync/Colors.qml.in
STAMP="$(date +%Y%m%d-%H%M%S)"

[ "$(id -u)" -eq 0 ] || { echo "run me with sudo" >&2; exit 1; }
[ -f "$TEMPLATE" ] || { echo "not found: $TEMPLATE" >&2; exit 1; }
command -v sddm-greeter-qt6 >/dev/null || { echo "sddm (qt6) is not installed" >&2; exit 1; }

echo "==> theme -> $THEME_DIR"
install -d -m 0755 "$THEME_DIR"
install -m 0644 "$SRC_DIR/theme/Main.qml" "$SRC_DIR/theme/metadata.desktop" "$THEME_DIR/"
# theme.conf carries the Background= line that sddm-theme-sync rewrites, so
# don't clobber a live one — only lay it down when the theme is new.
[ -f "$THEME_DIR/theme.conf" ] || install -m 0644 "$SRC_DIR/theme/theme.conf" "$THEME_DIR/"

# The theme cannot load without a Colors.qml, and matugen has not necessarily
# run yet. Seed a neutral grey render from the template — structurally identical
# to it, so sddm-theme-sync will accept the real render when it arrives.
if [ ! -f "$THEME_DIR/Colors.qml" ]; then
	echo "==> seeding a neutral Colors.qml (matugen will replace it)"
	sed -E 's/"\{\{[^"]*\}\}"/"#808080"/g' "$TEMPLATE" > "$THEME_DIR/Colors.qml"
	chmod 644 "$THEME_DIR/Colors.qml"
fi

echo "==> sync tool -> /usr/local/bin/sddm-theme-sync"
install -m 0755 -o root -g root "$SRC_DIR/sddm-theme-sync" /usr/local/bin/sddm-theme-sync

echo "==> validation reference -> $REFERENCE"
install -D -m 0644 -o root -g root "$TEMPLATE" "$REFERENCE"

echo "==> sudoers rule -> /etc/sudoers.d/sddm-theme-sync"
visudo -c -q -f "$SRC_DIR/sudoers.d-sddm-theme-sync"
install -m 0440 -o root -g root "$SRC_DIR/sudoers.d-sddm-theme-sync" /etc/sudoers.d/sddm-theme-sync

echo "==> crash-resilient session entry -> /usr/local/share/wayland-sessions/mango.desktop"
# Shadows the package's /usr/share/wayland-sessions/mango.desktop: /usr/local
# precedes /usr/share in SDDM's SessionDir search, and pacman never touches
# /usr/local, so this survives a mangowm-git upgrade. Same file name, so
# SDDM's remembered-last-session still resolves it with no action needed at
# the greeter. Exec launches mango through mango/scripts/session.sh, which
# restarts the compositor in place instead of dropping to this login screen
# on a crash — see plans/iterative-watching-spring.md.
install -D -m 0644 -o root -g root "$SRC_DIR/mango.desktop" \
	/usr/local/share/wayland-sessions/mango.desktop

echo "==> /etc/sddm.conf"
[ -f /etc/sddm.conf ] && cp -a /etc/sddm.conf "/etc/sddm.conf.bak.$STAMP"
install -m 0644 -o root -g root "$SRC_DIR/sddm.conf" /etc/sddm.conf

cat <<EOF

Installed. Next:

  ~/.config/mango/scripts/switchwall.sh --noswitch     # pull in the real colours
  sddm-greeter-qt6 --test-mode --theme $THEME_DIR      # look at it before trusting it
  sudo systemctl restart sddm                          # with a TTY available

Rollback:
  sudo cp /etc/sddm.conf.bak.$STAMP /etc/sddm.conf
  sudo rm /usr/local/share/wayland-sessions/mango.desktop
EOF
