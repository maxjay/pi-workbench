#!/usr/bin/env bash
# Capture the real native editor under an isolated Linux display.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
output=${1:-"$repo/artifacts/pi-workbench.png"}
mkdir -p "$(dirname "$output")"
output=$(realpath "$output")
binary="$repo/target/debug/pi-workbench"
test -x "$binary"

preview_tmp=$(mktemp -d)
trap 'rm -rf "$preview_tmp"' EXIT
export XDG_DATA_HOME="$preview_tmp/data"
export XDG_CONFIG_HOME="$preview_tmp/config"
export XDG_CACHE_HOME="$preview_tmp/cache"
export XDG_RUNTIME_DIR="$preview_tmp/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export LIBGL_ALWAYS_SOFTWARE=1
export GDK_BACKEND=x11
export WINIT_UNIX_BACKEND=x11

python3 - "$repo" <<'PY'
import json
import os
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
digest = 0xcbf29ce484222325
for byte in os.fsencode(root):
    digest = ((digest ^ byte) * 0x100000001b3) & 0xffffffffffffffff
path = Path(os.environ['XDG_DATA_HOME']) / 'piworkbench' / 'workspaces' / f'{digest:016x}.json'
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps({
    'version': 1,
    'explorer_width': 240,
    'split': True,
    'split_fraction': 0.5,
    'open_files': ['apps/desktop/src/main.rs', 'apps/desktop/src/commands.rs', 'Cargo.toml'],
    'active': ['apps/desktop/src/main.rs', 'apps/desktop/src/commands.rs'],
}))
PY

timeout 45s xvfb-run -a -s '-screen 0 1280x800x24' bash -c '
    set -euo pipefail
    "$1" "$2" >"$3.log" 2>&1 &
    app_pid=$!
    trap '\''kill "$app_pid" 2>/dev/null || true'\'' EXIT
    visible=false
    for attempt in {1..40}; do
        kill -0 "$app_pid" || { cat "$3.log"; exit 1; }
        if xdotool search --onlyvisible --name "^Pi Workbench$" >/dev/null 2>&1; then
            visible=true
            break
        fi
        sleep 0.5
    done
    "$visible" || { cat "$3.log"; exit 1; }
    # Let asynchronous file restoration and the first GPU frames finish.
    sleep 4
    kill -0 "$app_pid"
    import -window root "$3"
    identify "$3"
' _ "$binary" "$repo" "$output"
