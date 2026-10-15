#!/usr/bin/env bash
# Installs Pi Workbench for the current user on WSL (WSLg) or desktop Linux.
#
#   ./install-wsl.sh                  # download the latest release with gh
#   ./install-wsl.sh pi-workbench     # install a binary you already have
#   ./install-wsl.sh release.tar.gz   # install from a downloaded release archive
#
# Installs to ~/.local/bin and adds a launcher, which WSLg also shows in the
# Windows Start menu. Never changes Pi's configuration.
set -euo pipefail

repo=maxjay/pi-workbench
bin_dir="$HOME/.local/bin"
apps_dir="$HOME/.local/share/applications"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*"; }

# 1. Get the binary.
source=${1:-}
if [[ -z "$source" ]]; then
    say "Downloading the latest release of $repo"
    if command -v gh >/dev/null; then
        gh release download --repo "$repo" --pattern 'pi-workbench-x86_64-linux.tar.gz' --dir "$work"
    elif command -v gh.exe >/dev/null; then
        # On WSL the Windows GitHub CLI works too, with a Windows path.
        gh.exe release download --repo "$repo" --pattern 'pi-workbench-x86_64-linux.tar.gz' \
            --dir "$(wslpath -w "$work")"
    else
        echo "gh is required to download from the private repository." >&2
        echo "Install it (https://cli.github.com), run 'gh auth login', or pass a downloaded file." >&2
        exit 1
    fi
    source="$work/pi-workbench-x86_64-linux.tar.gz"
fi
if [[ "$source" == *.tar.gz ]]; then
    tar -xzf "$source" -C "$work"
    binary="$work/pi-workbench/pi-workbench"
else
    binary="$source"
fi
[[ -f "$binary" ]] || { echo "No binary at $binary" >&2; exit 1; }

# 2. Runtime libraries (present on most WSLg images).
ldconfig=$(command -v ldconfig || echo /sbin/ldconfig)
# Read once: piping into `grep -q` can fail with SIGPIPE under pipefail.
libraries=$("$ldconfig" -p)
missing=()
for pair in libgtk-3.so.0:libgtk-3-0 libxkbcommon.so.0:libxkbcommon0 \
    libxkbcommon-x11.so.0:libxkbcommon-x11-0 libwayland-client.so.0:libwayland-client0 \
    libEGL.so.1:libegl1; do
    grep -q "${pair%%:*}" <<<"$libraries" || missing+=("${pair##*:}")
done
if ((${#missing[@]})); then
    say "Installing runtime libraries: ${missing[*]}"
    sudo apt-get install -y "${missing[@]}"
fi

# 3. Install.
mkdir -p "$bin_dir" "$apps_dir"
install -m 755 "$binary" "$bin_dir/pi-workbench"
cat >"$apps_dir/pi-workbench.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Pi Workbench
Comment=Code editor with Pi
Exec=$bin_dir/pi-workbench %f
Terminal=false
Categories=Development;TextEditor;
EOF
say "Installed $bin_dir/pi-workbench"
case ":$PATH:" in
*":$bin_dir:"*) ;;
*) warn "$bin_dir is not on PATH. Add: export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
esac

# 4. Check Pi. It must be the Linux install: a Windows pi under /mnt/c
#    cannot work with Linux paths.
pi_path=$(command -v pi || true)
node_version=$(node --version 2>/dev/null || true)
node_ok() { [[ "$(printf '%s\n' v22.19.0 "$1" | sort -V | head -1)" == v22.19.0 ]]; }
if [[ -z "$pi_path" || "$pi_path" == /mnt/* ]]; then
    warn "Pi is not installed inside Linux${pi_path:+ (found the Windows one at $pi_path)}."
    echo "    Install it with: npm install -g @earendil-works/pi-coding-agent"
    echo "    then sign in by running 'pi' once in a terminal."
fi
if [[ -z "$node_version" ]] || ! node_ok "$node_version"; then
    warn "Pi needs Node.js 22.19 or newer (found ${node_version:-none})."
    echo "    For example: curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash - && sudo apt-get install -y nodejs"
fi
if [[ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]]; then
    warn "No display found. On WSL, update with 'wsl --update' in Windows to get WSLg."
fi

say "Done. Open a project with: pi-workbench ~/path/to/project"
