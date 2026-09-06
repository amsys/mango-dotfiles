#!/usr/bin/env bash
# Verifies an install. Read-only: it writes nothing and it needs no sudo.
#
# Usage:
#   ./install-check.sh       report what is wrong
#   ./install-check.sh -v    also print each check that passes
#
# Exit code 0 when every check passes, 1 when one fails. A warning does not
# fail the run: it marks something optional, or something this script cannot
# see from an unprivileged shell.
#
# The expectations below are written out here on purpose, instead of read from
# install-config.sh. A check that asks the installer what it did can only ever
# agree with it. When the two drift apart, this script must fail and say so.
#
# The one exception is the root installers under system/. Each of those
# declares its own `# check:` line, because the file that installs a thing is
# the right place to record how you prove the thing landed.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
XDG_STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"

VERBOSE=0
[[ "${1:-}" == "-v" || "${1:-}" == "--verbose" ]] && VERBOSE=1

PASSED=0
WARNED=0
FAILED=0

sect() { printf '\n-- %s --\n' "$*"; }
ok() {
	PASSED=$((PASSED + 1))
	((VERBOSE)) && printf '  ok    %s\n' "$*"
	return 0
}
bad() {
	FAILED=$((FAILED + 1))
	printf '  FAIL  %s\n' "$*"
}
warn() {
	WARNED=$((WARNED + 1))
	printf '  warn  %s\n' "$*"
}
info() { printf '  %s\n' "$*"; }

# first_few LIST... — the first three, then a count of the rest.
first_few() {
	if (($# > 3)); then
		printf '%s %s %s and %d more' "$1" "$2" "$3" "$(($# - 3))"
	else
		printf '%s' "$*"
	fi
}

# ---------------------------------------------------------------- symlinks --

# check_tree SRC_DIR DEST_DIR LABEL — every tracked file must be a symlink
# back to this repo. Anything else means an edit here does not reach the
# desktop, which is the whole point of the install.
check_tree() {
	local src_dir="$1" dest_dir="$2" label="$3"
	local file rel dest total=0 good=0 missing=()
	[[ -d "$src_dir" ]] || {
		warn "$label  not in the repo"
		return 0
	}
	while IFS= read -r -d '' file; do
		rel="${file#"$src_dir"/}"
		[[ "$rel" == *.example ]] && continue
		total=$((total + 1))
		dest="$dest_dir/$rel"
		if [[ -L "$dest" && "$(readlink -f "$dest")" == "$(readlink -f "$file")" ]]; then
			good=$((good + 1))
		else
			missing+=("$rel")
		fi
	done < <(find "$src_dir" -type f -print0)
	if ((good == total)); then
		ok "$label  $good/$total linked"
	else
		bad "$label  $good/$total linked — $(first_few "${missing[@]}")"
	fi
}

sect "config symlinks"
for dir in mango kitty rofi matugen wlogout fish fontconfig git hypr; do
	check_tree "$REPO/$dir" "$XDG_CONFIG_HOME/$dir" "$dir/"
done
check_tree "$REPO/ironbar/scripts" "$XDG_CONFIG_HOME/ironbar/scripts" "ironbar/scripts/"
if [[ -L "$XDG_CONFIG_HOME/starship.toml" ]] &&
	[[ "$(readlink -f "$XDG_CONFIG_HOME/starship.toml")" == "$REPO/starship.toml" ]]; then
	ok "starship.toml"
else
	bad "starship.toml not linked"
fi

# ------------------------------------------------------- per-machine files --

sect "per-machine files"
for f in mango/theme.json mango/local.conf git/config.local; do
	if [[ -e "$XDG_CONFIG_HOME/$f" ]]; then
		ok "$f"
	else
		bad "$f missing — install-config.sh seeds it from the .example"
	fi
done

GITLOCAL="$XDG_CONFIG_HOME/git/config.local"
if [[ -f "$GITLOCAL" ]]; then
	if git config --file "$GITLOCAL" user.email >/dev/null 2>&1 &&
		git config --file "$GITLOCAL" user.name >/dev/null 2>&1; then
		ok "git/config.local has a name and an email"
	else
		bad "git/config.local has no user.name or user.email — git cannot commit"
	fi
fi

LOCALCONF="$XDG_CONFIG_HOME/mango/local.conf"
if [[ -f "$LOCALCONF" ]]; then
	# Only a live setting counts. The stock file explains the rule in a
	# comment, and that comment names $HOME itself.
	if grep -q '^[^#]*\$HOME' "$LOCALCONF"; then
		bad "mango/local.conf has a literal \$HOME — mango does not expand it"
	else
		ok "mango/local.conf has no unexpanded \$HOME"
	fi
fi

# ---------------------------------------------------------- built artifacts --

sect "built artifacts"
BARD="$HOME/.local/bin/mango-bard"
if [[ ! -e "$BARD" ]]; then
	bad "mango-bard missing from ~/.local/bin — the bar has no dynamic content"
elif [[ -L "$BARD" ]]; then
	# The 2026-09-06 outage. cargo's target dir is tmpfs on this machine, so a
	# link into it is empty after a reboot: ironbar exits 127, mango-bard
	# 203/EXEC, both restart for ever without reaching 'failed', and no bar
	# starts. install-config.sh must produce a real binary, never a link.
	bad "mango-bard is a symlink to $(readlink "$BARD") — the build dir is
        temporary and is empty after a reboot. Re-run install-config.sh."
elif [[ ! -x "$BARD" ]]; then
	bad "mango-bard is not executable"
elif [[ -n "$(find "$REPO/ironbar/bard/src" -name '*.rs' -newer "$BARD" 2>/dev/null)" ]]; then
	warn "mango-bard is older than ironbar/bard/src — re-run install-config.sh"
else
	ok "mango-bard is a real binary and up to date"
fi
command -v mango-bard >/dev/null 2>&1 || warn "$HOME/.local/bin is not on PATH"

SHIM="$HOME/.local/lib/mango/fast-tooltips.so"
if [[ -f "$SHIM" ]]; then
	ok "fast-tooltips.so"
else
	warn "fast-tooltips.so missing — tooltips keep the 500 ms GTK3 delay"
fi

# ------------------------------------------------------ systemd user units --

# Started at login by mango's config.conf exec-once. Enabled AND running is
# the only healthy state for these while a session is up.
ENABLED_UNITS=(mango-bard.service ironbar.service mango-sleep-lock.service
	mango-powerkey.service mango-outputs.service)
# Linked but never enabled: a bar toggle starts them, never the login.
ONDEMAND_UNITS=(wayvnc.service wayvnc-privacy.service kdeconnectd.service
	mango-keepawake.service)

sect "systemd user units"
if ! command -v systemctl >/dev/null 2>&1 ||
	! systemctl --user show-environment >/dev/null 2>&1; then
	warn "no user systemd bus — run this inside the desktop session"
else
	for u in "${ENABLED_UNITS[@]}"; do
		if [[ ! -L "$XDG_CONFIG_HOME/systemd/user/$u" ]]; then
			bad "$u not linked into ~/.config/systemd/user"
			continue
		fi
		en=$(systemctl --user is-enabled "$u" 2>/dev/null || true)
		ac=$(systemctl --user is-active "$u" 2>/dev/null || true)
		if [[ "$en" != enabled && "$en" != enabled-runtime ]]; then
			bad "$u is '$en', expected enabled"
		elif [[ "$ac" == active ]]; then
			ok "$u  enabled, active"
		else
			bad "$u  enabled but $ac — config.conf's exec-once names it, so
        it should be running. Check: journalctl --user -b -u $u"
		fi
	done
	for u in "${ONDEMAND_UNITS[@]}"; do
		if [[ ! -L "$XDG_CONFIG_HOME/systemd/user/$u" ]]; then
			bad "$u not linked into ~/.config/systemd/user"
			continue
		fi
		en=$(systemctl --user is-enabled "$u" 2>/dev/null || true)
		if [[ "$en" == enabled || "$en" == enabled-runtime ]]; then
			warn "$u is enabled — it should start from a bar toggle only"
		else
			ok "$u  linked, on demand ($en)"
		fi
	done

	if [[ -L "$XDG_CONFIG_HOME/systemd/user/arch-update.timer.d/mango.conf" ]]; then
		if [[ "$(systemctl --user show arch-update.timer -p Persistent --value 2>/dev/null)" == yes ]]; then
			ok "arch-update.timer override in effect"
		else
			warn "arch-update.timer override linked but not loaded — daemon-reload"
		fi
	else
		warn "arch-update.timer override not linked"
	fi
fi

sect "kdeconnect overrides"
for pair in \
	"$REPO/kdeconnect/org.kde.kdeconnect.daemon.desktop:$XDG_CONFIG_HOME/autostart/org.kde.kdeconnect.daemon.desktop" \
	"$REPO/kdeconnect/org.kde.kdeconnect.service:$XDG_DATA_HOME/dbus-1/services/org.kde.kdeconnect.service"; do
	src="${pair%%:*}"
	dst="${pair#*:}"
	if [[ -L "$dst" && "$(readlink -f "$dst")" == "$(readlink -f "$src")" ]]; then
		ok "$(basename "$dst")"
	else
		bad "$(basename "$dst") not overridden — kdeconnectd starts outside the toggle"
	fi
done

# ------------------------------------------------------------- theme output --

sect "generated theme output"
GEN="$XDG_STATE_HOME/mango/generated"
GEN_N=$(find "$GEN" -type f 2>/dev/null | wc -l)
if ((GEN_N > 0)); then
	ok "$GEN  ($GEN_N files)"
else
	bad "$GEN is empty — run mango/scripts/switchwall.sh --noswitch"
fi

# --------------------------------------------------------- root installers --

# Each system/<name>/install.sh says how to prove it landed, in its own
# `# check:` line. Absent is not a failure: every one of them is optional.
sect "root installers (system/)"
while IFS= read -r script; do
	name=$(basename "$(dirname "$script")")
	probe=$(awk '/^# check: /{print substr($0, 10); exit}' "$script")
	case "$probe" in
	"")
		warn "$name  declares no '# check:' line"
		;;
	ufw:*)
		rule="${probe#ufw:}"
		if [[ ! -r /etc/ufw/user.rules ]]; then
			info "$(printf '%-14s ?  needs root to read /etc/ufw/user.rules' "$name")"
		elif grep -q "$rule" /etc/ufw/user.rules; then
			info "$(printf '%-14s ok installed' "$name")"
		else
			info "$(printf '%-14s -- not installed' "$name")"
		fi
		;;
	*)
		if [[ -e "$probe" ]]; then
			info "$(printf '%-14s ok %s' "$name" "$probe")"
		else
			info "$(printf '%-14s -- not installed' "$name")"
		fi
		;;
	esac
done < <(find "$REPO/system" -mindepth 2 -maxdepth 2 -name install.sh | sort)

# ------------------------------------------------------------ machine state --

# The parts of docs/install.md's manual checklist that a script can see.
sect "machine state"
if [[ -e "$HOME/.gitconfig" ]]; then
	bad "$HOME/.gitconfig exists — it shadows the tracked ~/.config/git/config"
else
	ok "no ~/.gitconfig shadowing the tracked config"
fi

if grep -rqs '^HandlePowerKey=ignore' \
	/etc/systemd/logind.conf.d/ /etc/systemd/logind.conf; then
	ok "logind HandlePowerKey=ignore"
else
	warn "logind does not set HandlePowerKey=ignore — powerkey.py is bypassed"
fi

for u in gnome-keyring-daemon.service gnome-keyring-daemon.socket; do
	st=$(systemctl --user is-enabled "$u" 2>/dev/null || true)
	if [[ -z "$st" || "$st" == masked ]]; then
		ok "$u  ${st:-not present}"
	else
		warn "$u is '$st' — mask it so KeePassXC owns the Secret Service"
	fi
done

if [[ -n "$(find "$HOME/Wallpapers" -maxdepth 1 -type f -print -quit 2>/dev/null)" ]]; then
	ok "$HOME/Wallpapers has images"
else
	warn "$HOME/Wallpapers is empty or missing — SUPER+W has nothing to switch to"
fi

# ------------------------------------------------------------------ summary --

printf '\n== %d ok, %d warn, %d FAIL ==\n' "$PASSED" "$WARNED" "$FAILED"
((VERBOSE)) || printf '(-v also lists the %d checks that pass)\n' "$PASSED"
((FAILED == 0))
