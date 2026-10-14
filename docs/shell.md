# Shell — fish, git and cargo

## fish

`fish/config.fish` defines abbreviations, not aliases. The list includes
`pi`, `pu`, `gs`, `gc` and `gd`. Run `abbr --show` to see all of them.

An abbreviation expands in place. You see the real command before you run
it, and you can edit it. The history records the command that ran. An alias
gives you none of this.

The config also starts four tools. These tools do nothing after a bare
install:

| Line | Gives you |
|---|---|
| `zoxide init fish \| source` | `z <partial-name>` jumps to a frequent directory. `zi` shows a list and lets you select one |
| `fzf_key_bindings` | `Ctrl+R` for fuzzy history, `Ctrl+T` to insert a path, `Alt+C` to cd into a subdirectory |
| `function y` | yazi. On exit, the shell moves to the directory you ended in |
| `MANPAGER` | man pages through `bat` |

`fzf` supplies its key-binding function, but it does not call the function.
Put the call in `config.fish`, not in `conf.d/`. Fish loads `conf.d/` first.
Fish's own binding setup then replaces the fzf bindings.

The `claude` abbreviations start Claude Code for a type of task. Type the
abbreviation and a space. The cursor stops inside the quotes. Type the task
there.

| Abbr | Use it for | Model, effort, mode | Why this tier |
|---|---|---|---|
| `cs` | A small fix or change | sonnet, default, acceptEdits | Median session 0.45 USD |
| `cw` | 1-6 steps, `/workflow-design` | opus, high, acceptEdits | Sonnet does not obey the gates of a skill |
| `ct` | A multi-task feature, `/tierplan` | fable, xhigh, acceptEdits | The session holds the design slot. A Workflow drives the waves |
| `cpl` | A plan with evidence before code | opus, high, plan | Opus planners find bugs before code |
| `cf` | A design, an audit, or a stuck problem | fable, xhigh, plan | A wrong decision is expensive to undo |
| `ch` | The `/handoff` menu | opus, high, default | The ideate, design and build stages run on Opus |
| `cr` | Resume a session | unchanged | |

The tiers come from `~/brain/Resources/model-tiers-guide.md`, section 7.

`opus` is the 200k window. The 1M window (`opus[1m]`) spent 67% of the
money, because a long session sends its full context again on each turn.
Type `opus[1m]` only when one session must hold a large corpus.

Keep Fable sessions short. Fable costs 0.94 USD for each turn before it
does work. Do not use `cf` or `ct` for menus, status checks or long
sessions.

Change the model, the effort or the mode on the line before you push enter.

`fish/conf.d/claude.fish` is untracked on purpose. It holds machine-local
API keys. See [security.md](security.md).

## git

`git/config` is the global git config. It moved here from `~/.gitconfig` to
keep it in the repo. It sets `git-delta` as the pager.

**The trap:** git reads `$XDG_CONFIG_HOME/git/config` first. Git reads
`~/.gitconfig` second, and the second file wins. The tracked file does
nothing until you delete `~/.gitconfig`. Install delta before you delete
`~/.gitconfig`. If you do not, every paged git command fails. `install.sh`
prints this as a manual step.

Put the per-machine identity (the name and the email) in
`git/config.local`. The installer seeds this file from
`config.local.example`. The file is untracked.

No delta theme is pinned. matugen controls the color, and delta falls back
to `$BAT_THEME`. `$BAT_THEME` is the extension point for theming.

## cargo

`$CARGO_HOME/config.toml` is `~/.local/share/cargo/config.toml`. It controls
every Rust build on this machine. The file is not in this repo, and no
installer writes it. Copy it by hand if you move to a new machine.

It sets three things.

**`build.target-dir = "/tmp/cargo-target"`.** All build output goes to tmpfs.
The root disk is a DRAM-less QLC NVMe. The parallel small writes of a build
raise its write latency to about 1 second and stall the machine. `rust-lld`
writes the most. Two results follow. `target/` is empty after each reboot,
and all projects share one directory. No script can read
`<crate>/target/release/`. Use `cargo install`, or ask cargo for the path.

**`build.jobs = 4`.** Each `rust-lld` process uses about 450 MB. `target/` is
RAM, so the build competes with the files that it writes. Remove the line for
full parallelism. Watch `df -h /tmp` when you do.

**Reduced debug info.** `[profile.dev]` sets `debug = "line-tables-only"`.
`[profile.dev.package."*"]` sets `debug = 0`. The cargo default is full DWARF.
Debug info was the largest part of the output. Clean builds of `mango-bard`
on 2026-09-08:

| Setting | Size | Time |
|---|---|---|
| `debug = 2` (cargo default) | 206 MB | 13.7 s |
| `debug = "line-tables-only"` | 154 MB | 12.4 s |
| plus `package."*"` `debug = 0` | 121 MB | 11.8 s |

### Debug info: what stays, and how to get more

You keep the file names and the line numbers. `RUST_BACKTRACE=1` and
`RUST_BACKTRACE=full` show the same frames as before. A panic still prints
`src/main.rs:1:14`, and each frame still prints its own `at` line.

You lose the variable data. `print x` in gdb or lldb finds nothing to read.

Do not edit the config file to debug. Set the value for one build:

```bash
# full debug info for your own code
CARGO_PROFILE_DEV_DEBUG=2 cargo build

# also for every dependency
cargo build --config 'profile.dev.package."*".debug=2'

# for one dependency only
cargo build --config 'profile.dev.package.tokio.debug=2'
```

A glob key has no environment-variable name. Use `--config` for the last two.

Measured effect of the first command on the `mango-bard` binary: the
`.debug_info` section goes from 1438685 bytes to 7214158 bytes. Read it with
`readelf -SW <binary> | grep debug_info`.

Each change of a debug level makes every cached artifact invalid. The first
build after a change is a full rebuild. This is correct. Nothing is broken.

## /tmp quota and tmp-sweep

`/tmp` is a tmpfs with a per-user quota. systemd 262 sets the quota at login.
The default is 80% of the size of `/tmp`. On this machine `/tmp` is 24 GiB.
The hard limit for user martin is 18.67 GiB.

`df` does not show the quota. `df` shows the free space of the whole tmpfs.
On 2026-10-13, `df` showed 4.8 GB free. Only 120 MB was left for the user.

### The symptom

When the quota is full, a new shell exits with status 1 and prints nothing.
Other tools fail too:

- The Claude Code Bash tool cannot write to `/tmp/claude-1000`.
- Headless Chromium stops at start.
- Builds fail with `Disk quota exceeded` or ENOSPC.

This happened on 2026-09-08 (twice) and on 2026-10-11. Each time, builds of
the `hx` project filled the quota. Those builds write to `/tmp/cargo-target`.
See the cargo section above.

### What tmp-sweep removes

`tmp-sweep` runs from `tmp-sweep.timer` every 10 minutes. The first run starts
5 minutes after boot.

Pressure is the used space divided by the quota limit. Pressure sets the
age limits:

| Pressure | Cargo artifacts not read for | Claude session dirs not used for |
|---|---|---|
| below 0.70 | 24 h | 72 h |
| 0.70 or more | 3 h | 24 h |

Cargo artifacts are in `/tmp/cargo-target`. A profile dir is a directory that
holds a `.cargo-lock` file. In each profile dir, the sweep removes files in
`deps/` and `examples/` one by one. It keeps a file that has more than one
hard link. In `build/`, `.fingerprint/` and `incremental/`, it removes whole
unit dirs. It never removes part of a unit.

Claude scratch dirs are `/tmp/claude-<uid>/<project>/<session-uuid>/`. The
sweep removes only dirs whose name is a UUID. Other names stay, for example
`bash-edit-diff/`.

Removing an artifact costs build time only. Cargo rebuilds a missing output.
rustc starts a new incremental session.

"Last use" of a file is the newer of its atime and mtime. `/tmp` mounts with
`strictatime`, so each read by cargo or rustc updates atime. "Last use" of a
directory is its mtime only. Each scan of a directory sets its atime to now,
and the sweep itself scans every unit dir. With atime, no unit dir would ever
become stale after the first run.

### The lock rule

Cargo holds a `flock` on `<profile>/.cargo-lock` for the whole build. Before
the sweep touches a profile, it takes the same lock without waiting. When a
build holds the lock, the sweep skips that profile and logs
`skip <profile>: build running`. A build that starts during the sweep waits.
The wait lasts until the sweep releases the lock for that profile.

### Live sessions stay

Claude Code writes a registry file for each session, at
`~/.claude/sessions/<pid>.json`. A session is live when its process exists
and its start time matches `procStart` in the file.

The sweep never removes the scratch dir of a live session, however old the
dir is. It also keeps a dir when a process of your user has it as its working
directory or holds a file open in it. When the registry cannot be listed, the
sweep skips all Claude scratch dirs for that run.

### Notification

At pressure 0.85 or more, the sweep sends one critical notification. The
notification names the three largest entries in `/tmp` that you own. The
sweep sends no second notification until pressure drops below 0.80.

### How to see what happened

- `journalctl --user -u tmp-sweep` shows one summary line for each run. The
  line gives the quota source, the used and limit bytes before and after the
  run, the pressure, the age limits, the bytes freed and the skipped profiles.
- `tmp-sweep --dry-run` prints each path that it would remove, with its size.
  It ends with a summary line that says "would free". It removes nothing and
  sends no notification.

When the quota call fails, the sweep uses 80% of the tmpfs size as the
limit. The summary line names the source that it used.

### Recovery when fresh artifacts fill the quota

The sweep cannot remove fresh files. Under pressure, the limits are 3 h for
cargo artifacts and 24 h for Claude scratch dirs. A build that writes now
keeps its files fresh.

1. Run `tmp-sweep --dry-run`. Read the pressure and the largest paths.
2. Stop the build that fills the quota. Run `du -sh /tmp/cargo-target/*` to
   find the largest profile.
3. Wait for the next run. Under pressure, files that are 3 h old are removed.
4. When `/tmp` is still full, remove a profile dir or one unit dir by hand.
   Do this only when no build runs in it. `flock -n <profile>/.cargo-lock true`
   exits 0 when the lock is free.
