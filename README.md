# Pi Workbench

[![CI](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml)

A small native code editor built with Rust and [Floem](https://github.com/lapce/floem).

**Status:** early prototype. Pi integration is planned.

## Features

- File explorer and tabbed editing
- Two editor panes with shared buffers and undo history
- Side-by-side changes since the last commit, editable in place
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
| Ctrl/Cmd+Shift+G | Toggle changes view |
| Ctrl/Cmd+Shift+P | Command palette |
| Escape | Dismiss palette or dialog |

## Development

| Source | Responsibility |
| --- | --- |
| [workspace.rs](apps/desktop/src/workspace.rs) | Opening files, pane selection, confirmations, layout |
| [documents.rs](apps/desktop/src/documents.rs) | Buffers, saves, reloads |
| [changes.rs](apps/desktop/src/changes.rs) | Comparing a document with a baseline, live diff refresh |
| [explorer.rs](apps/desktop/src/explorer.rs) | Directory loading and cache |
| [commands.rs](apps/desktop/src/commands.rs) | Actions and shortcuts |
| [ui/](apps/desktop/src/ui) | Views and styling |
| [workbench-core](crates/workbench-core/src) | File operations, layout storage, line diffs, baseline sources |

To add an action, update `Command`, `COMMANDS`, `Command::details`, and `execute` in `commands.rs`. Keep its behavior in the module that owns the relevant state.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

### Comparison sources

The changes view compares an open document with a [`BaselineSource`](crates/workbench-core/src/baseline.rs). The built-in `GitHead` source reads the committed file with `git cat-file --filters`, so line endings match the working copy. To compare with something else, such as the index, another branch, or a snapshot taken before Pi edits a file, implement the trait and pass it to `Workbench::new`. Sources run off the UI thread and return `None` for files that are new.

The working-copy side is the shared document buffer, so edits, undo, and saves behave as in a normal tab. Diffs refresh shortly after each edit; [diff.rs](crates/workbench-core/src/diff.rs) has no UI dependencies.

## Current limitations

- UTF-8 files only; 4 MiB per file, 64 open documents
- No syntax highlighting, LSP, terminal, file watching, or Pi integration yet
- Tab changes reset cursor and scroll position
- The changes view compares one file with HEAD; the baseline scrolls with the working copy instead of padding removed or added lines
- Workspace layouts are saved on normal close; unsaved edits have no crash recovery
