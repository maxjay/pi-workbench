#!/bin/sh
# Builds wbd inside WSL and puts it where the app and `npm run dev` look for
# it: ~/.local/bin/wbd. Run it from the repo, in WSL:
#
#   wsl -e sh -l scripts/wbd.sh
#
# The build goes to a folder inside WSL, because building on the Windows
# drive from Linux is slow.
set -e
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/workbench-target}"
cargo build -p wbd
mkdir -p "$HOME/.local/bin"
install -m 755 "$CARGO_TARGET_DIR/debug/wbd" "$HOME/.local/bin/wbd"
echo "wbd $("$HOME/.local/bin/wbd" --version) is in ~/.local/bin"
