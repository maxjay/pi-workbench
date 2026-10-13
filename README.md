# Pi Workbench

A native Rust + Floem editor prototype, built as the first step toward a lightweight IDE with an independent Pi daemon.

## Run

Install [Rust via rustup](https://rustup.rs/). The repository pins the toolchain and Floem revision and includes `Cargo.lock`.

```sh
git clone https://github.com/maxjay/pi-workbench.git
cd pi-workbench
cargo run --release --locked -p pi-workbench -- /path/to/project
```

Omit the folder to open the current directory. On macOS, install the Xcode command-line tools (`xcode-select --install`); on Windows, use the MSVC Rust toolchain and Visual Studio C++ Build Tools. Linux requires GTK 3 and XKB development libraries:

```sh
sudo apt-get install build-essential pkg-config libgtk-3-dev libxkbcommon-dev libxkbcommon-x11-dev
```

Run inside a graphical desktop session. WSL needs WSLg or another configured display server.

## What works

- Open folders in separate native windows; expand a lazy, virtualized file explorer.
- Open UTF-8 files in tabs. Both panes can display and edit the **same document and undo history**, or different files.
- Save asynchronously, with dirty indicators, permission-preserving atomic replacement, and external-change detection before saving.
- Close-file and close-window prompts protect unsaved work. Reload from disk asks before discarding edits.
- Drag the explorer divider and editor divider. Hide the explorer, move it to the other side, or toggle the second pane.
- Restore open files, active files, panel positions, and split sizes per workspace on normal shutdown.
- Use a searchable command palette backed by one internal command registry.

| Shortcut | Action |
| --- | --- |
| Ctrl/Cmd+S | Save active file |
| Ctrl/Cmd+W | Close active file |
| Ctrl/Cmd+Q | Close workspace window |
| Ctrl/Cmd+B | Toggle explorer |
| Ctrl/Cmd+\ | Toggle second pane |
| Ctrl/Cmd+Shift+P | Commands |
| Escape | Dismiss command palette or confirmation |

Select a pane before opening a file to choose its destination. Closing a tab closes that document in both panes. The close confirmation's **Save all** saves files; **Close when saved** closes only once all required saves have finished. A failed save keeps the document open.

## Scope and performance

This is the editor foundation. Pi integration, terminal, LSP, syntax highlighting, Git UI, file watchers, arbitrary docking, Vim mode, and runtime plugins are **not implemented yet**. No Node runtime or agent process is started.

The UI has two persistent editor views and one rope-backed document per open file. File operations run on short-lived background threads; expanded directory results are cached; idle application code has no polling loop. Only visible file rows are mounted. Closing a document releases its reactive scope. A saved text baseline is retained for conflict checks.

Limits for this milestone: 4 MiB per file, 64 open documents, UTF-8 text only; explorer skips symlinks, `.git`, `target`, and `node_modules`. Unsaved buffers are not crash recovery backups. Layout is written on normal close, not continuously. Tab changes reset cursor and scroll position. Undo back to the original contents can leave a conservative dirty indicator until Save is pressed. Refresh and Reload are explicit; external edits are detected at save time.

Atomic replacement prevents partial file contents, but the comparison before replacement is not a cross-process transaction. A concurrent writer can still race the final check. Saves preserve basic file permissions; extended attributes and ACL preservation require further work. Hard-linked files are rejected rather than breaking their link relationship.

No target-machine CPU, memory, or latency budgets have been established yet. Benchmark release builds on the actual Mac/Windows/Linux setup, reporting the complete process tree and GPU resources. Headless software rendering is suitable for functional checks, not representative performance claims.

## Structure

| Location | Responsibility |
| --- | --- |
| `apps/desktop/src/ui.rs` | Native panels, editor views, resizers, dialogs |
| `apps/desktop/src/state.rs` | Document ownership, async I/O, workspace lifecycle |
| `apps/desktop/src/commands.rs` | Internal command contributions shared by toolbar and palette |
| `crates/workbench-core` | Renderer-independent file safety and layout persistence |

Layout metadata lives in the OS's application-data directory, keyed by canonical workspace path. It contains file paths and layout values, not document contents. A corrupt or unsupported layout is preserved and not overwritten automatically.

Floem is pinned to [`1351ffb`](https://github.com/lapce/floem/tree/1351ffb162faeb8be983f1301d718a0d5fb59c27). Its built-in editor provides the rope, editing commands, selection, and undo. The workbench owns document lifetimes and disk state. Native services remain usable independently of any future agent.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

CI contains Linux, macOS, and Windows jobs. A workflow being present does not mean those platforms have all been tested. Local validation on Linux passed the build, formatting, Clippy, and eight core tests. Native UI smoke checks covered shared editing, saving, external-change refusal, reload confirmation, resizing, and layout restoration.

Core tests exercise external-change conflicts, deleted/binary files, symlink escape protection, permissions, Unicode/CRLF saves, and layout restoration/sanitization. To check the UI, open the same file in both panes, edit in one, save, edit the file externally, verify a conflicting save is refused, then adjust the layout and reopen the workspace.

## Next milestones

1. Incremental syntax highlighting and per-tab view state; event-driven file watching with explicit conflict resolution.
2. Terminal, lazy LSP services, search, and Git integration, each with a measured resource budget.
3. A separate Rust Pi supervisor: private local IPC, stable session IDs, bounded on-demand Pi RPC workers, event sequence numbers, and reconnect snapshots. Closing the workbench must not terminate agent work.
4. Durable session metadata and honest interrupted-run recovery. Do not automatically replay ambiguous tool calls.
5. Route supported Pi extension questions/approvals to native UI. Add compiled Rust panel/command contributions; evaluate a dynamic plugin host only when needed.

For restricted corporate builds, mirror the locked registry and Git dependencies or run `cargo vendor --locked` in an approved connected environment. The native workbench itself does not require model-provider access.

