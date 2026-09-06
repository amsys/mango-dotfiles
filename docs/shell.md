# Shell — fish and git

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
