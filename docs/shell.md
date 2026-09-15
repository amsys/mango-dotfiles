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
