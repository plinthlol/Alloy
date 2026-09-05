#!/usr/bin/env fish


set -l name "Alloy"
set -l uuid "e1438414-64d6-4592-9769-d073419a8d1e"
scoped to the block, so declare
# config_dir up here and assign inside.
set -l config_dir
if set -q XDG_CONFIG_HOME
    set config_dir "$XDG_CONFIG_HOME/alloy"
else
    set config_dir "$HOME/.config/alloy"
end
set -l accounts_file "$config_dir/accounts.json"

if not mkdir -p "$config_dir"
    echo "Failed to create config directory $config_dir" >&2
    exit 1
end

# a single account as the app would serialize it
function fresh_account_json
    printf '[{"uuid":"%s","username":"%s","account_type":"Offline","active":true}]' $argv[1] $argv[2]
end

if command -v jq >/dev/null 2>&1
    if test -f "$accounts_file"; and test -s "$accounts_file"
        # merge into the existing list: drop any account with the same uuid,
        # then append the new one, active iff the list ended up empty.
        jq --arg uuid "$uuid" --arg name "$name" '
            map(select(.uuid != $uuid)) as $rest
            | $rest + [{uuid: $uuid, username: $name, account_type: "Offline", active: ($rest | length == 0)}]
        ' "$accounts_file" > "$accounts_file.tmp"
        and mv "$accounts_file.tmp" "$accounts_file"
        or begin
            rm -f "$accounts_file.tmp"
            echo "Failed to merge into $accounts_file (is it valid JSON?)" >&2
            exit 1
        end
    else
        fresh_account_json "$uuid" "$name" > "$accounts_file"
    end
else
    # no jq: safe for a fresh/empty accounts file, but refuse to clobber an
    # existing list without a JSON parser (it may hold Microsoft refresh tokens).
    if test -f "$accounts_file"; and test -s "$accounts_file"
        echo "accounts.json already exists and jq is not installed." >&2
        echo "Install jq and re-run, or add the account from the TUI (Accounts -> a -> o)." >&2
        exit 1
    end
    fresh_account_json "$uuid" "$name" > "$accounts_file"
end

if test -s "$accounts_file"
    chmod 600 "$accounts_file"
    echo "Added offline account '$name' ($uuid) to $accounts_file"
else
    echo "Failed to write $accounts_file" >&2
    exit 1
end
