#!/usr/bin/env bash
# Sets material-code.primaryColor in every installed editor's settings.json.
# jq + tmp + mv (switchwall.sh's own pattern for theme.json), not sed: the
# old add-key sed only produced valid JSON when the file's last line was
# exactly "}" — a single-line "{}" file got corrupted — and its cleanup sed
# could never match, since sed's pattern space is one line and a literal
# \n never appears in it. jq handles the has-the-key and doesn't-have-it
# cases identically (`.foo = $v` sets or replaces), and jq's own parse
# failure (this file may carry VS Code's JSONC comments, which jq can't
# read) means nothing is written rather than a corrupted file.
COLOR_FILE_PATH="${XDG_STATE_HOME:-$HOME/.local/state}/mango/generated/color.txt"

# Define an array of possible VSCode settings file paths for various forks
settings_paths=(
    "${XDG_CONFIG_HOME:-$HOME/.config}/Code/User/settings.json"
    "${XDG_CONFIG_HOME:-$HOME/.config}/VSCodium/User/settings.json"
    "${XDG_CONFIG_HOME:-$HOME/.config}/Code - OSS/User/settings.json"
    "${XDG_CONFIG_HOME:-$HOME/.config}/Code - Insiders/User/settings.json"
    "${XDG_CONFIG_HOME:-$HOME/.config}/Cursor/User/settings.json"
    "${XDG_CONFIG_HOME:-$HOME/.config}/Antigravity/User/settings.json"

    # Add more paths as needed for other forks
)

new_color=$(cat "$COLOR_FILE_PATH" 2>/dev/null)
if [[ -z "$new_color" ]]; then
    echo "vscode-set-color: $COLOR_FILE_PATH missing or empty, nothing to set" >&2
    exit 1
fi

# Loop through each settings file path
for CODE_SETTINGS_PATH in "${settings_paths[@]}"; do
    if [[ -f "$CODE_SETTINGS_PATH" ]]; then
        tmp=$(mktemp "$CODE_SETTINGS_PATH.XXXXXX") || continue
        if jq --arg color "$new_color" '.["material-code.primaryColor"] = $color' \
            "$CODE_SETTINGS_PATH" >"$tmp"; then
            mv "$tmp" "$CODE_SETTINGS_PATH"
        else
            echo "vscode-set-color: skipping $CODE_SETTINGS_PATH — jq could not parse it" >&2
            rm -f "$tmp"
        fi
    fi
done

