#!/usr/bin/env bash
# Status lines for the hyprlock lock screen, and the TPM verdict that gates
# fingerprint login there. hyprlock's cmd[] labels (matugen/templates/
# hyprlock/hyprlock.conf) run `line` and `caps`; sleep-lock.py reads the
# verdict file that `tpm` and `line` write.
#
#   lock-status.sh tpm         check the TPM now; print ok, fail or none
#   lock-status.sh line RRGGBB the date line of the boot screen: the TOTP
#                              code, the TPM warning in colour RRGGBB (no
#                              "#": hyprlang reads it as a comment), or the
#                              date when the TOTP cannot be read
#   lock-status.sh caps        "Caps Lock is on", or nothing
#
# The TOTP comes from `sudo -n /usr/bin/tpm2-totp calculate`. The sudoers
# rule (system/lockscreen/sudoers.d-mango-totp) allows exactly that command:
# it prints the code for this 30 s slot, never the secret.
#
# Verdicts:
#   ok    the TPM unsealed the secret: the boot chain matches the seal
#   fail  the TPM refused (sealed PCRs changed): do not trust this boot
#   none  no sudo rule or no TPM: nothing is known
#
# $XDG_RUNTIME_DIR/mango-lock/tpm-state holds "<boot_id> <verdict>" for ok
# and fail only. sleep-lock.py turns fingerprint login on only when it
# reads "<this boot_id> ok". Each `line` call for a new slot rewrites it.
set -u

dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/mango-lock"
boot=$(cat /proc/sys/kernel/random/boot_id 2>/dev/null) || boot=unknown

# save FILE TEXT: write via a temp file and rename, so a reader never sees
# half a line.
save() {
	mkdir -m 0700 "$dir" 2>/dev/null
	[[ -d $dir ]] || return 0
	printf '%s\n' "$2" >"$dir/$1.tmp" 2>/dev/null && mv -f "$dir/$1.tmp" "$dir/$1"
}

# calculate: print the 6-digit code and record ok, or print fail and record
# fail. Return 1 and print nothing when the sudo rule is missing (verdict
# none: the old verdict is removed, not kept).
calculate() {
	local out
	out=$(sudo -n /usr/bin/tpm2-totp calculate 2>&1)
	if [[ $out =~ ^[0-9]{6}$ ]]; then
		save tpm-state "$boot ok"
		printf '%s' "$out"
		return 0
	fi
	# sudo's own refusals ("sudo: a password is required") start with
	# "sudo:"; tpm2-totp's errors start with "ERROR". `sudo -n -l` is no
	# test for the rule: the plain wheel rule lists the command too.
	if [[ $out == sudo:* ]]; then
		rm -f "$dir/tpm-state"
		return 1
	fi
	save tpm-state "$boot fail"
	printf 'fail'
}

cmd_tpm() {
	local code
	if ! code=$(calculate); then
		echo none
	elif [[ $code == fail ]]; then
		echo fail
	else
		echo ok
	fi
}

# One TPM call per 30 s slot. hyprlock runs this every second; the other
# 29 calls read the cache.
cmd_line() {
	local err=${1:-ff0000} slot cached code
	slot=$(($(date +%s) / 30))
	cached=$(cat "$dir/code" 2>/dev/null) || cached=
	if [[ ${cached%% *} == "$boot:$slot" ]]; then
		code=${cached#* }
	else
		code=$(calculate) || code=none
		save code "$boot:$slot $code"
	fi
	case "$code" in
	fail) printf '<span foreground="#%s">TPM mismatch: do not enter your password</span>\n' "$err" ;;
	none) date '+%A, %-d %B' ;;
	# U+F132 shield, the boot screen's TOTP icon (plymouth-theme-sync).
	*) printf '\xef\x84\xb2  %s %s\n' "${code:0:3}" "${code:3}" ;;
	esac
}

# The kernel LED follows the compositor's lock state on every keyboard.
cmd_caps() {
	local f
	for f in /sys/class/leds/*::capslock/brightness; do
		[[ -r $f && $(<"$f") != 0 ]] && { echo 'Caps Lock is on'; return 0; }
	done
	echo ''
}

case "${1:-}" in
tpm) cmd_tpm ;;
line) cmd_line "${2:-}" ;;
caps) cmd_caps ;;
*)
	echo "usage: lock-status.sh tpm | line RRGGBB | caps" >&2
	exit 2
	;;
esac
