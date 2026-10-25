#!/usr/bin/env bash
set -euo pipefail

maintenance="$(realpath "${1:?maintenance binary path is required}")"
workspace="$(mktemp -d -t orbit-packaging-test-XXXXXXXX)"
trap 'rmdir "$workspace/home/.config" "$workspace/home/.local/share" "$workspace/home/.local" "$workspace/home/.cache" "$workspace/home" "$workspace" 2>/dev/null || true' EXIT
mkdir -p "$workspace/home/.config" "$workspace/home/.local/share" "$workspace/home/.cache"
export HOME="$workspace/home"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_CACHE_HOME="$HOME/.cache"

# A dry run must resolve current-user XDG paths without creating business files.
for level in cache settings all; do
    "$maintenance" --level "$level" --dry-run --output-format json | python3 -c '
import json, os, sys
plan = json.load(sys.stdin)
assert plan["roots"]["config"] == os.environ["XDG_CONFIG_HOME"]
assert plan["roots"]["data"] == os.environ["XDG_DATA_HOME"]
assert plan["roots"]["cache"] == os.environ["XDG_CACHE_HOME"]
assert not plan["data_entries"] and not plan["game_entries"]
'
done
if "$maintenance" --yes </dev/null; then
    echo 'Noninteractive cleanup must not silently select a level' >&2
    exit 1
fi
if find "$HOME" -type f -print -quit | grep -q .; then
    echo 'Dry-run maintenance must not create per-user files' >&2
    exit 1
fi
printf '%s\n' 'Linux packaging/current-user cleanup contract verified.'
