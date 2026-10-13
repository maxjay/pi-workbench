# Pi Workbench

[![CI](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml)

A small native code editor built with Rust and [Floem](https://github.com/lapce/floem).

**Status:** early prototype. Pi integration is planned.

## Features

- File explorer and tabbed editing
- Two editor panes with shared buffers and undo history
- Background saves with external-change detection
- Unsaved-change prompts before closing or reloading
- Resizable panels and saved workspace layouts
- Command palette and keyboard shortcuts

## Getting started

Install [Rust](https://rustup.rs). The repository pins the toolchain.

Platform prerequisites:

- **macOS:** Xcode command-line tools (`xcode-select --install`)
- **Windows:** Visual Studio C++ Build Tools and the MSVC Rust toolchain
- **Debian/Ubuntu:** `sudo apt install build-essential pkg-config libgtk-3-dev libxkbcommon-dev libxkbcommon-x11-dev`

```sh
git clone https://github.com/maxjay/pi-workbench.git
cd pi-workbench
cargo run --release --locked -p pi-workbench -- /path/to/project
```

Omit the path to open the current directory. A graphical desktop is required; WSL needs WSLg.

## Shortcuts

Use Ctrl on Windows/Linux or Cmd on macOS.

| Shortcut | Action |
| --- | --- |
| Ctrl/Cmd+S | Save file |
| Ctrl/Cmd+W | Close file |
| Ctrl/Cmd+Q | Close window |
| Ctrl/Cmd+B | Toggle explorer |
| Ctrl/Cmd+\ | Toggle second pane |
| Ctrl/Cmd+Shift+P | Command palette |
| Escape | Dismiss palette or dialog |

## Development

| Source | Responsibility |
| --- | --- |
| [workspace.rs](apps/desktop/src/workspace.rs) | Opening files, pane selection, confirmations, layout |
| [documents.rs](apps/desktop/src/documents.rs) | Buffers, saves, reloads |
| [explorer.rs](apps/desktop/src/explorer.rs) | Directory loading and cache |
| [commands.rs](apps/desktop/src/commands.rs) | Actions and shortcuts |
| [ui/](apps/desktop/src/ui) | Views and styling |
| [workbench-core](crates/workbench-core/src) | File operations and layout storage |

To add an action, update `Command`, `COMMANDS`, `Command::details`, and `execute` in `commands.rs`. Keep its behavior in the module that owns the relevant state.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

## Current limitations

- UTF-8 files only; 4 MiB per file, 64 open documents
- No syntax highlighting, LSP, terminal, file watching, or Pi integration yet
- Tab changes reset cursor and scroll position
- Workspace layouts are saved on normal close; unsaved edits have no crash recovery
