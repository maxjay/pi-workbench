# Pi Workbench

[![CI](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml)

A small native code editor built with Rust and [Floem](https://github.com/lapce/floem).

**Status:** early prototype. Works with the [Pi](https://pi.dev) coding agent.

## Features

- File explorer and tabbed editing
- Two editor panes with shared buffers and undo history
- Side-by-side changes since the last commit, editable in place
- Pi runs in the background; its edits stream into open files without overwrite prompts
- Background saves with external-change detection
- Unsaved-change prompts before closing or reloading
- Resizable panels and saved workspace layouts
- Command palette and keyboard shortcuts

## Install on WSL or Linux

Release builds come from [the Release workflow](.github/workflows/release.yml) for every `v*` tag. Inside WSL (Windows 11 or Windows 10 with WSLg):

```sh
gh release download --repo maxjay/pi-workbench --pattern '*linux.tar.gz'
tar -xzf pi-workbench-x86_64-linux.tar.gz
./pi-workbench/install-wsl.sh pi-workbench-x86_64-linux.tar.gz
pi-workbench ~/code/my-project
```

[install-wsl.sh](scripts/install-wsl.sh) installs to `~/.local/bin`, adds a launcher that WSLg also lists in the Windows Start menu, installs missing runtime libraries with `apt`, and checks that Pi is installed inside Linux with Node.js 22.19 or newer. Run it without arguments to download the latest release itself (with `gh`, or `gh.exe` from Windows). Without a GPU driver the app draws with the CPU; set `FLOEM_FORCE_TINY_SKIA=1` to force that if rendering looks wrong.

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

## Using Pi

Install Pi (`npm install -g @earendil-works/pi-coding-agent`) and sign in to a provider as usual. Open the Pi panel with Ctrl/Cmd+J and type a request. The workbench starts `pi --mode rpc` in the workspace folder, so Pi uses your normal settings, credentials, extensions, and sessions; nothing is installed into Pi.

- Open files are saved before each prompt, so Pi reads what you see.
- While the model writes an edit, its tool card shows the change line by line.
- When Pi changes a file you have open, the change is folded into the tab. Unsaved edits are merged; if you and Pi changed the same lines, both versions stay in the file between conflict markers. There is never a reload or overwrite prompt, and the next save works normally.
- Lines Pi changed stay highlighted until you type.

To pass extra arguments, set `PI_WORKBENCH_PI_ARGS`, for example `--provider <name> --model <id>`. `PI_WORKBENCH_PI` selects a different `pi` executable.

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
| Ctrl/Cmd+J | Toggle Pi panel |
| Ctrl/Cmd+Shift+P | Command palette |
| Escape | Dismiss palette or dialog |

## Development

| Source | Responsibility |
| --- | --- |
| [workspace.rs](apps/desktop/src/workspace.rs) | Opening files, pane selection, confirmations, layout |
| [documents.rs](apps/desktop/src/documents.rs) | Buffers, saves, reloads |
| [changes.rs](apps/desktop/src/changes.rs) | Comparing a document with a baseline, live diff refresh |
| [agent.rs](apps/desktop/src/agent.rs) | Running Pi, its activity log, and reporting file changes |
| [explorer.rs](apps/desktop/src/explorer.rs) | Directory loading and cache |
| [commands.rs](apps/desktop/src/commands.rs) | Actions and shortcuts |
| [ui/](apps/desktop/src/ui) | Views and styling |
| [workbench-core](crates/workbench-core/src) | File operations, layout storage, line diffs, merges, baseline sources |
| [pi-client](crates/pi-client/src) | Starting Pi in RPC mode, decoding its events, previewing streamed edits |

To add an action, update `Command`, `COMMANDS`, `Command::details`, and `execute` in `commands.rs`. Keep its behavior in the module that owns the relevant state.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

### Testing with a fake model

[scripts/fake-model.mjs](scripts/fake-model.mjs) is a scripted OpenAI-compatible server for running Pi without a provider. A prompt that is a JSON array of steps drives it; see the file header. Point Pi at it with a separate agent directory so your own configuration is untouched:

```sh
node scripts/fake-model.mjs 47124 &
mkdir -p /tmp/pi-fake && echo '{"providers":{"fake":{"baseUrl":"http://127.0.0.1:47124/v1","api":"openai-completions","apiKey":"fake","models":[{"id":"fake"}]}}}' > /tmp/pi-fake/models.json
PI_CODING_AGENT_DIR=/tmp/pi-fake PI_WORKBENCH_PI_ARGS="--provider fake --model fake --no-session" cargo run -p pi-workbench
```

`PI_CLIENT_LIVE=1 cargo test -p pi-client` runs the same setup against the installed `pi`.

### Comparison sources

The changes view compares an open document with a [`BaselineSource`](crates/workbench-core/src/baseline.rs). The built-in `GitHead` source reads the committed file as a raw blob; it never runs the repository's filter commands, and line-ending differences are ignored when diffing. To compare with something else, such as the index, another branch, or a snapshot taken before Pi edits a file, implement the trait and set it in the `Integrations` passed to `Workbench::new` (see [ui/mod.rs](apps/desktop/src/ui/mod.rs)). Sources run off the UI thread and return `None` for files that are new.

The working-copy side is the shared document buffer, so edits, undo, and saves behave as in a normal tab. Diffs refresh shortly after each edit; [diff.rs](crates/workbench-core/src/diff.rs) has no UI dependencies.

## Current limitations

- UTF-8 files only; 4 MiB per file, 64 open documents
- No syntax highlighting, LSP, terminal, or file watching; changes Pi makes to files that are not open appear when you open them
- Tab changes reset cursor and scroll position
- The changes view compares one file with HEAD; the baseline scrolls with the working copy instead of padding removed or added lines
- Workspace layouts are saved on normal close; unsaved edits have no crash recovery
