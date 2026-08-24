# Shell — fish and git

## fish

`fish/config.fish` defines abbreviations, not aliases (`pi`, `pu`, `gs`,
`gc`, `gd`, and the rest — `abbr --show` lists them). An abbreviation
expands in place: you see the real command before you run it, you can edit
it, and history records what actually ran. An alias gives you none of that.

The config also wires up four tools that do nothing after a bare install:

| Line | Gives you |
|---|---|
| `zoxide init fish \| source` | `z <partial-name>` jumps to a frequent directory; `zi` picks interactively |
| `fzf_key_bindings` | `Ctrl+R` fuzzy history, `Ctrl+T` insert a path, `Alt+C` cd into a subdirectory |
| `function y` | yazi that leaves the shell in the directory you ended in |
| `MANPAGER` | man pages through `bat` |

`fzf` ships its key-binding function and never calls it. The call must live
in `config.fish`, not `conf.d/` — `conf.d/` loads first and fish's own
binding setup would override it.

`fish/conf.d/claude.fish` is untracked on purpose: it holds machine-local
API keys. See [security.md](security.md).

## git

`git/config` is the global git config, moved here from `~/.gitconfig` so it
lives in the repo. It sets `git-delta` as the pager.

**The trap:** git reads `$XDG_CONFIG_HOME/git/config` first and
`~/.gitconfig` second, and the later file wins. The tracked file does
nothing until `~/.gitconfig` is deleted. Install delta before you delete
it, or every paged git command fails. `install.sh` prints this as a manual
step.

Per-machine identity (name, email) goes in `git/config.local`, seeded from
`config.local.example` and untracked.

No delta theme is pinned. matugen owns color, and delta falls back to
`$BAT_THEME` — that is the hook if theming is ever needed.
