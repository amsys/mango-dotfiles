#!/usr/bin/env bash
# Front end for the install. It asks before each step, and it lists the root
# installers under system/ so you pick only the ones this machine needs.
#
# Four steps, in order. Each one also runs on its own:
#
#   ./install-deps.sh              report the missing packages (read-only)
#   ./install-config.sh            symlink the repo into ~/.config
#   sudo system/<name>/install.sh  one root installer
#   ./install-check.sh             verify the result (read-only)
#
# Usage:
#   ./install.sh                     ask before each step
#   ./install.sh --yes               no questions: deps + config, no root installers
#   ./install.sh --no                no questions: decline every step
#   ./install.sh --system=sddm,rapl  run those root installers, show no menu
#   ./install.sh --system=all        run every root installer
#   ./install.sh --check             the verify step alone
#   ./install.sh --dry-run           print every action, change nothing
#   ./install.sh --skip              never replace a file that already exists
#   ./install.sh --force             replace an existing file with no backup
#
# --skip and --force pass through to install-config.sh. A root installer
# accepts neither: each one decides for itself, and each is safe to re-run.
#
# The exit code is the exit code of the verify step. 0 means the install
# checks out.
set -euo pipefail

log() { printf '%s\n' "$*"; }
die() {
	printf 'install.sh: %s\n' "$*" >&2
	exit 2
}

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"

ASSUME=ask # ask | yes | no
CHECK_ONLY=0
DRY_RUN=0
SYSTEM_ARG=
SYSTEM_GIVEN=0
CONFIG_ARGS=()

for arg in "$@"; do
	case "$arg" in
	--yes | -y) ASSUME=yes ;;
	--no | -n) ASSUME=no ;;
	--check) CHECK_ONLY=1 ;;
	--dry-run)
		DRY_RUN=1
		CONFIG_ARGS+=(--dry-run)
		;;
	--skip | --force) CONFIG_ARGS+=("$arg") ;;
	--system=*)
		SYSTEM_ARG="${arg#--system=}"
		SYSTEM_GIVEN=1
		;;
	-h | --help)
		awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' \
			"${BASH_SOURCE[0]}"
		exit 0
		;;
	*) die "unknown option $arg (try --help)" ;;
	esac
done

# Every system/<name>/install.sh, in name order. Each script declares two
# things about itself in its own header, so this menu needs no list to keep in
# step: `# check:` (how install-check.sh proves it landed) and `# risk: boot`
# (it changes what boots, or how you log in). Add an installer and it appears
# here with no edit to this file.
SYS_NAMES=()
SYS_DIRS=()
while IFS= read -r script; do
	SYS_DIRS+=("$script")
	SYS_NAMES+=("$(basename "$(dirname "$script")")")
done < <(find "$REPO/system" -mindepth 2 -maxdepth 2 -name install.sh | sort)

# describe SCRIPT — the first sentence of the header comment, for the menu.
# It stops at a period followed by a space, so a name like llama.cpp or
# mango-powerkey.service does not cut the line in half.
describe() {
	awk '
		NR == 1 && /^#!/ { next }
		/^# *(check|risk):/ { next }
		/^#/ {
			line = $0
			sub(/^# ?/, "", line)
			buf = (buf == "" ? line : buf " " line)
			if (match(buf, /\. /)) exit
			next
		}
		{ exit }
		END {
			if (match(buf, /\. /)) buf = substr(buf, 1, RSTART)
			if (length(buf) > 72) buf = substr(buf, 1, 69) "..."
			print buf
		}
	' "$1"
}

is_risky() { grep -q '^# risk: boot' "$1"; }

# ask PROMPT — yes or no, default yes. --yes and --no answer without asking.
ask() {
	local reply
	case "$ASSUME" in
	yes)
		log "$1 [Y/n] y"
		return 0
		;;
	no)
		log "$1 [Y/n] n"
		return 1
		;;
	esac
	read -r -p "$1 [Y/n] " reply
	[[ -z "$reply" || "$reply" == [Yy]* ]]
}

show_menu() {
	local i mark
	log "  These need sudo. Each is safe to re-run. None of them is required."
	log
	for i in "${!SYS_NAMES[@]}"; do
		mark=" "
		is_risky "${SYS_DIRS[$i]}" && mark="!"
		printf '  %s %2d  %-14s %s\n' \
			"$mark" "$((i + 1))" "${SYS_NAMES[$i]}" "$(describe "${SYS_DIRS[$i]}")"
	done
	log
	log "  ! changes what boots or how you log in. Confirmed separately."
}

# select_spec SPEC — fills SELECTED with menu indexes. SPEC is empty, 'all',
# or menu numbers and names separated by spaces or commas.
SELECTED=()
select_spec() {
	local spec="$1" tok i found seen
	SELECTED=()
	[[ -z "${spec//[[:space:]]/}" ]] && return 0
	if [[ "$spec" == all ]]; then
		for i in "${!SYS_NAMES[@]}"; do SELECTED+=("$i"); done
		return 0
	fi
	for tok in ${spec//,/ }; do
		found=
		if [[ "$tok" =~ ^[0-9]+$ ]] && ((tok >= 1 && tok <= ${#SYS_NAMES[@]})); then
			found=$((tok - 1))
		else
			for i in "${!SYS_NAMES[@]}"; do
				[[ "${SYS_NAMES[$i]}" == "$tok" ]] && found="$i"
			done
		fi
		[[ -n "$found" ]] || {
			printf 'no such installer: %s\n' "$tok" >&2
			return 1
		}
		# Do not run one twice. plymouth rebuilds the initramfs, which is slow.
		for seen in ${SELECTED[@]+"${SELECTED[@]}"}; do
			[[ "$seen" == "$found" ]] && found=
		done
		[[ -n "$found" ]] && SELECTED+=("$found")
	done
	return 0
}

run_system() {
	local i name risky=() keep=() ok=() failed=()
	((${#SELECTED[@]})) || {
		log "  none chosen."
		return 0
	}

	for i in "${SELECTED[@]}"; do
		is_risky "${SYS_DIRS[$i]}" && risky+=("$i")
	done

	if ((${#risky[@]})); then
		log
		log "  These change what boots or how you log in:"
		for i in "${risky[@]}"; do log "    sudo ${SYS_DIRS[$i]}"; done
		log "  A bad result here can leave the machine with no boot and no"
		log "  login screen. Read the script first if you are not sure."
		if ! confirm_boot; then
			for i in "${SELECTED[@]}"; do
				is_risky "${SYS_DIRS[$i]}" || keep+=("$i")
			done
			SELECTED=(${keep[@]+"${keep[@]}"})
			log "  left out the ones marked ! — the rest still run."
		fi
	fi

	((${#SELECTED[@]})) || {
		log "  nothing left to run."
		return 0
	}

	if ((DRY_RUN)); then
		for i in "${SELECTED[@]}"; do
			log "  (dry run) would run: sudo ${SYS_DIRS[$i]}"
		done
		return 0
	fi

	sudo -v || {
		log "  no sudo — the root installers did not run."
		return 0
	}

	for i in "${SELECTED[@]}"; do
		name="${SYS_NAMES[$i]}"
		log
		log "== $name =="
		if sudo "${SYS_DIRS[$i]}"; then
			ok+=("$name")
		else
			failed+=("$name")
			log "  !! $name FAILED. The remaining ones still run."
		fi
	done
	log
	((${#ok[@]})) && log "  ran: ${ok[*]}"
	((${#failed[@]})) && log "  FAILED: ${failed[*]}"
	return 0
}

confirm_boot() {
	local reply
	case "$ASSUME" in
	yes)
		log "  --yes, and you named them: running them."
		return 0
		;;
	no) return 1 ;;
	esac
	read -r -p "  type 'yes' to run these as well: " reply
	[[ "$reply" == yes ]]
}

# A name typed wrong must fail now, not after the config install. This runs
# before the terminal check below, so a typo reports the typo.
if ((SYSTEM_GIVEN)) && ! select_spec "$SYSTEM_ARG"; then
	die "--system takes 'all' or any of: ${SYS_NAMES[*]}"
fi

# Steps 1 and 2 ask a question, and a question needs somewhere to read the
# answer from. --check asks nothing.
if [[ "$ASSUME" == ask ]] && ((!CHECK_ONLY)) && [[ ! -t 0 ]]; then
	die "not a terminal — pass --yes, --no or --check"
fi

log "== mango-dotfiles install =="
log "repo:   $REPO"
log "target: $XDG_CONFIG_HOME"
((DRY_RUN)) && log "(dry run — nothing changes)"
log

if ((CHECK_ONLY)); then
	exec "$REPO/install-check.sh"
fi

log "-- 1/4  packages --"
ask "report the missing packages?" && "$REPO/install-deps.sh"
log

log "-- 2/4  config into $XDG_CONFIG_HOME --"
ask "symlink the config?" &&
	"$REPO/install-config.sh" ${CONFIG_ARGS[@]+"${CONFIG_ARGS[@]}"}
log

log "-- 3/4  root installers (system/) --"
if ((SYSTEM_GIVEN)); then
	run_system
elif [[ "$ASSUME" == ask ]]; then
	show_menu
	read -r -p "  choose (numbers, names, 'all', or empty to skip): " reply
	if select_spec "$reply"; then run_system; fi
else
	log "  skipped. Pass --system=<names> or --system=all to run them."
fi
log

CHECK_RC=0
log "-- 4/4  verify --"
if ((DRY_RUN)); then
	log "  (dry run) would run: ./install-check.sh"
else
	"$REPO/install-check.sh" || CHECK_RC=$?
fi
log

log "Manual steps that no script can do:"
log "  - KeePassXC: the entry holding the OpenRouter key must carry the attribute"
log "    application=mango (was: illogical-impulse). Edit it under Advanced ->"
log "    Additional attributes, or rofi's Alt+I returns 'not found'."
log "  - fish/conf.d/claude.fish is NOT installed (contained a live API key on"
log "    the source machine) — recreate it from the keyring, not from history"
log "  - git: delete ~/.gitconfig once git-delta is installed. It shadows the"
log "    tracked ~/.config/git/config (git reads XDG first, \$HOME second, later"
log "    wins), so until it is gone none of the delta settings apply."
log "  - Review mango/local.conf and mango/theme.json for values specific to the"
log "    OLD machine (monitor name, XDG_DATA_DIRS, wallpaper path)"
log "  - After system/fprint-notify/install.sh, add the PAM line it prints to"
log "    /etc/pam.d/sudo by hand. No script edits a PAM auth stack."
log "  - See docs/install.md for the full checklist of manual steps"
log
log "Re-run './install-check.sh' at any time to see what is still missing."

exit "$CHECK_RC"
