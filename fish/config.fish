# Commands to run in interactive sessions can go here
if status is-interactive
    # No greeting
    set fish_greeting

    # Use starship
    function starship_transient_prompt_func
        starship module character
    end
    if test "$TERM" != "linux"
        starship init fish | source
        enable_transience
    end

    # Tool integration. These packages were installed but never hooked up —
    # each of the three lines below is the whole reason the package is on disk.

    # zoxide: `z proj` jumps to the most-used dir matching "proj", `zi` picks
    # one interactively (it shells out to fzf, also below).
    command -q zoxide && zoxide init fish | source

    # fzf ships fzf_key_bindings as a vendor function but never calls it, so a
    # stock install has no bindings at all. This gives Ctrl-R (fuzzy history),
    # Ctrl-T (insert a path), Alt-C (cd into a subdir). Must come after any
    # conf.d/ key-binding files, which load first — hence config.fish, not
    # conf.d/.
    functions -q fzf_key_bindings && fzf_key_bindings

    # Point fzf's own pickers at fd: respects .gitignore, and reaches dotfiles,
    # which the default `find` invocation does not.
    set -x FZF_DEFAULT_COMMAND 'fd --type f --hidden --exclude .git'
    set -x FZF_CTRL_T_COMMAND $FZF_DEFAULT_COMMAND
    set -x FZF_ALT_C_COMMAND 'fd --type d --hidden --exclude .git'

    # Syntax-highlighted man pages. col -bx strips the backspace-overstrike
    # bolding that groff emits and bat would otherwise render literally.
    set -x MANPAGER "sh -c 'col -bx | bat -l man -p'"
    set -x MANROFFOPT -c

    # yazi, but the directory you quit in becomes the shell's directory —
    # without this wrapper yazi is just a viewer. Named `y` rather than
    # shadowing `yazi`, same reasoning as the abbr block below.
    function y --wraps yazi --description 'yazi, cd to wherever you quit'
        set -l tmp (mktemp -t "yazi-cwd.XXXXXX")
        yazi $argv --cwd-file="$tmp"
        if read -z cwd <"$tmp"; and test -n "$cwd"; and test "$cwd" != "$PWD"
            builtin cd -- "$cwd"
        end
        rm -f -- "$tmp"
    end

    # Aliases
    # kitty doesn't clear properly so we need to do this weird printing
    alias clear "printf '\033[2J\033[3J\033[1;1H'"
    alias celar "printf '\033[2J\033[3J\033[1;1H'"
    alias claer "printf '\033[2J\033[3J\033[1;1H'"
    alias pamcan pacman
    if test "$TERM" != "linux"
        alias ls 'eza --icons=auto'
    end

    # Abbreviations, not aliases: an abbr expands in place, so the real command
    # stays visible before you hit enter, lands in history as what actually ran,
    # and is editable (add -y, change a flag). It also cannot shadow a binary.
    # Chosen from what this machine's history actually shows, most-used first.

    # packages
    abbr --add pi sudo pacman -S
    abbr --add pu sudo pacman -Syu
    abbr --add prm sudo pacman -Rns
    abbr --add pss pacman -Ss
    abbr --add pq pacman -Q
    abbr --add ys yay -Ss
    abbr --add yi yay -S
    abbr --add yr yay -Rns

    # git
    abbr --add gs git status
    abbr --add gc git commit
    abbr --add gp git push
    abbr --add gd git diff
    abbr --add gl git log --oneline -20
    abbr --add gsw git switch
    abbr --add gcl git clone

    # systemd
    abbr --add scu systemctl --user
    abbr --add scs sudo systemctl
    abbr --add jcu journalctl --user -u

    # misc
    abbr --add cr claude --resume
    abbr --add tf tail -f
    abbr --add duh du -sh \* \| sort -h
    abbr --add psg ps aux \| grep -v grep \| grep
end
