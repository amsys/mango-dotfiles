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

## Known trade-offs

- `system/rapl/` opens the RAPL energy counters to the `wheel` group. RAPL
  is root-only upstream because of the PLATYPUS side channel
  (CVE-2020-8694). On a single-user laptop that attack does not apply, and
  RAPL is the only root-free source of per-domain power data.
- `wayvnc.service` listens on `0.0.0.0:5900` when started. It stays
  disabled; the bar's remote toggle starts it on demand, and the firewall
  is expected to restrict the port.
