# Security

## Secrets

This repo tracks no credentials. Do not add any. Keep the secrets in the
KeePassXC keyring. `mango/scripts/keyring-lookup.sh` reads them at runtime
through the Secret Service. `rofi/ai.sh` is the worked example.

Two files stay untracked because they hold machine-local values.
`fish/conf.d/claude.fish` exports API keys into the shell.
`git/config.local` holds the name and the email. `.gitignore` lists both
files. It also lists the `*.kdbx`, `*.key`, `*.pem` and `.env` catch-alls.

The catch-alls are necessary because this repo is symlinked into
`~/.config`. An application can write a new file into a tracked directory
at any time. Without the catch-alls, `git add -A` adds that file.

`keyring-lookup.sh` examines the lock state of the collection *before* it
calls `secret-tool`. A lookup on a locked collection stops at the keyring
unlock prompt. A detached worker has no terminal and cannot answer it.

## The sudo helper pattern

Some features need root at runtime. These are the login-screen colors, the
boot palette and the CPU power knobs. All of them use one pattern:

- A `system/*/install.sh` script installs the helper root-owned in
  `/usr/local/bin`. The user can never write to that file.
- One sudoers rule permits exactly one verb with **no free arguments**.
  The data comes in on stdin or through a fixed file path.
- The helper validates all data that it receives before it acts:
  - `sddm-theme-sync` installs a rendered QML file only if the file
    differs from a root-owned reference by color literals. It refuses a
    structural change, because the greeter executes this file before
    login.
  - `plymouth-theme-sync` accepts nine fixed `<role> <rrggbb>` lines and
    one wallpaper path. It renders each PNG itself and packs them into
    `/boot/mango-plymouth.img`. No user-written file goes to `/boot`
    unchanged. One risk stays, and this repo accepts it: plymouthd
    decodes those PNGs as root in the initramfs. The inputs are the same
    files that SDDM already shows, and libpng is the only parser.
  - `mango-powermode` validates each `KEY=value` from stdin against a
    closed set before it writes to sysfs. It never reads the
    user-writable config file.

This pattern replaces the usual alternative. That alternative is a
`NOPASSWD` rule that points to a script in `$HOME`. Such a rule gives
`NOPASSWD: ALL` in practice. The user can rewrite the script, and so can
any program that runs as the user.

## Fingerprint prompt notification

`system/fprint-notify/` shows a mako notification. The notification has a
fingerprint glyph and the name of the program that asked. The user then
always knows why the reader came on.

`pam_exec.so` is the only mechanism that carries the identity of the
program that asked. It exports `PAM_SERVICE` to the program that it runs.
A D-Bus watch on fprintd cannot do this. An unprivileged user cannot
become a bus monitor. The fprintd API also does not tell "sudo" from
"swaylock".

The helper runs as root in the auth stack of sudo. For this reason it
never uses `set -e`. It always exits 0. It hands the notification to a
detached `runuser` child. A stuck notification daemon must not delay a
sudo password prompt.

`/etc/pam.d/swaylock` stays excluded on purpose. swaylock covers the whole
screen. A notification behind swaylock is not visible.

`install.sh` does not edit `/etc/pam.d/sudo`. Open a root shell in a
second terminal first. Then add this line by hand, above the existing
`auth sufficient pam_fprintd.so` line:

```
auth optional pam_exec.so quiet /usr/local/bin/mango-fprint-notify
```

## Known trade-offs

- `system/rapl/` opens the RAPL energy counters to the `wheel` group.
  Upstream keeps RAPL root-only because of the PLATYPUS side channel
  (CVE-2020-8694). That attack does not apply to a single-user laptop.
  RAPL is also the only source of per-domain power data that does not
  need root.
- `wayvnc.service` listens on `[::]:5900` when it starts. That socket is
  dual-stack and also serves IPv4. The service has no auth and no
  encryption. It stays disabled, and the remote toggle on the bar starts
  it on demand. Only ufw (`system/remote/install.sh`) limits the port to
  wg_hetzner and the mango hotspot. See `system/remote/README.md`.
