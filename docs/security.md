# Security

## Secrets

No credentials are tracked here, and none should ever be. Secrets belong in
the KeePassXC keyring. `mango/scripts/keyring-lookup.sh` reads them at
runtime through the Secret Service; `rofi/ai.sh` is the worked example.

Two files are untracked because they hold machine-local values:
`fish/conf.d/claude.fish` (API keys exported into the shell) and
`git/config.local` (name and email). Both are in `.gitignore`, next to
`*.kdbx` / `*.key` / `*.pem` / `.env` catch-alls. The catch-alls matter
because this repo is symlinked into `~/.config`: an application can drop a
new file into a tracked directory at any time, and `git add -A` would
otherwise sweep it up.

`keyring-lookup.sh` checks the collection's lock state *before* it calls
`secret-tool`. A lookup against a locked collection blocks on an unlock
prompt, and a detached worker has no terminal to answer one.

## The sudo helper pattern

Several features need root at runtime (login-screen colors, boot palette,
CPU power knobs). All of them follow one pattern:

- The helper is installed root-owned in `/usr/local/bin` by a
  `system/*/install.sh` script. It is never a file the user can write.
- One sudoers rule allows exactly one verb with **no free arguments**.
  Data travels on stdin or through a fixed file path.
- The helper validates everything it receives before it acts:
  - `sddm-theme-sync` installs a rendered QML file only when it differs
    from a root-owned reference by color literals alone. It refuses
    structural changes, because the greeter executes this file before
    login.
  - `console-palette-sync` accepts only exactly 16 valid
    `<index> <rrggbb>` lines before it writes `/boot/mango-palette.img`.
  - `mango-powermode` re-validates every `KEY=value` from stdin against a
    closed set before it touches sysfs, and it never reads the
    user-writable config file itself.

The pattern exists because the common alternative — a `NOPASSWD` rule that
points at a script inside `$HOME` — is `NOPASSWD: ALL` in practice: the
user (or anything running as the user) can rewrite the script.

## Fingerprint prompt notification

`system/fprint-notify/` shows a mako notification with a fingerprint glyph
and the name of whatever asked, so a reader that lights up on its own is
always explained.

`pam_exec.so` is the only mechanism that carries the asker's identity. It
exports `PAM_SERVICE` to the program it runs; a D-Bus watch on fprintd
cannot do this, because an unprivileged user cannot become a bus monitor,
and fprintd's API has no concept of "sudo" against "swaylock" anyway.

The helper runs as root inside the auth stack of sudo, so it never uses
`set -e`, always exits 0, and hands the notification to a detached
`runuser` child. A wedged notification daemon must not delay a sudo
password prompt.

`/etc/pam.d/swaylock` is deliberately excluded. swaylock covers the whole
screen, so a notification behind it is invisible.

`install.sh` does not edit `/etc/pam.d/sudo`. Add this line by hand, above
the existing `auth sufficient pam_fprintd.so` line, with a root shell open
in a second terminal:

```
auth optional pam_exec.so quiet /usr/local/bin/mango-fprint-notify
```

## Known trade-offs

- `system/rapl/` opens the RAPL energy counters to the `wheel` group. RAPL
  is root-only upstream because of the PLATYPUS side channel
  (CVE-2020-8694). On a single-user laptop that attack does not apply, and
  RAPL is the only root-free source of per-domain power data.
- `wayvnc.service` listens on `[::]:5900` (dual-stack, also serves v4)
  when started, with no auth and no encryption. It stays disabled; the
  bar's remote toggle starts it on demand, and ufw
  (`system/remote/install.sh`) is the only thing restricting the port to
  wg_hetzner and the mango hotspot — see `system/remote/README.md`.
