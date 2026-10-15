# Pi Workbench

[![CI](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml)

A native workbench for the [Pi](https://pi.dev) coding agent, built with Rust and [Floem](https://github.com/lapce/floem). Pi sessions are the top level: tab between them, read what each one did, and edit the files it touches without overwrite prompts.

**Status:** early prototype.

![Two Pi sessions, one resumed with its history](docs/screenshots/sessions.png)

## Features

- **Sessions:** every Pi session is a tab in the sessions rail. Sessions resume from Pi's own store with their history, so the Pi command line and the workbench see the same sessions.
- **Conversation:** replies render as markdown; tool calls are cards that show each edit line by line while the model writes it; extension questions are answered inline; steer a running session or queue a follow-up.
- **Changes lens:** everything is read against one baseline you pick in the status bar: where your branch left develop, the start of the session, Pi's last turn, or the last commit. The explorer marks changed files, editors show removed lines as ghost lines in place, and you walk the changes file to file without opening a diff.
- **Who changed it:** changed lines are coloured by who wrote them: Pi, you, or someone else on the branch. Reformatting is filtered out, and the changed files are grouped by the Pi turn that changed them.
- **No overwrite prompts:** Pi's edits fold into open files. Unsaved edits are merged; overlapping changes stay in the file between conflict markers.
- **Editor:** syntax highlighting, open by name (Ctrl+P), search across files (Ctrl+Shift+F), cursor and scroll kept per tab, and an explorer that hides what `.gitignore` ignores.
- A second pane, background saves, and a command palette.

## Install on WSL or Linux

Release builds come from [the Release workflow](.github/workflows/release.yml) for every `v*` tag. Inside WSL (Windows 11 or Windows 10 with WSLg):

```sh
curl -fsSL -o pw.tar.gz https://github.com/maxjay/pi-workbench/releases/latest/download/pi-workbench-x86_64-linux.tar.gz
tar -xzf pw.tar.gz && ./pi-workbench/install-wsl.sh pw.tar.gz
pi-workbench ~/code/my-project
```

[install-wsl.sh](scripts/install-wsl.sh) installs to `~/.local/bin`, adds a launcher that WSLg also lists in the Windows Start menu, installs missing runtime libraries with `apt`, and checks that Pi is installed inside Linux with Node.js 22.19 or newer. On WSL the app renders with Mesa's software OpenGL, because WSLg's GPU-backed OpenGL fails during setup; set `PI_WORKBENCH_GPU=1` to try the GPU anyway.

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

## Working with Pi

Install Pi (`npm install -g @earendil-works/pi-coding-agent`) and sign in to a provider as usual. Each session runs its own `pi --mode rpc` in the workspace folder, so Pi uses your normal settings, credentials, extensions, and session store; nothing is installed into Pi.

- **Sessions rail:** open sessions with their status (ready, working, needs you, stopped), and recent sessions from Pi's store for this folder. Closing a session stops its Pi process; it stays in Recent.
- **Composer:** Enter sends, Shift+Enter adds a line. While Pi works, Enter steers it and Alt+Enter queues a follow-up for when it finishes.
- **Files:** open files are saved before each prompt, so Pi reads what you see. While Pi runs, saves wait and then go through the same merge, so neither side is lost. Clicking an edit card opens the file next to the conversation; lines Pi changed stay highlighted until you type.
- **Header:** session name, status, tokens, cost, context use, and the model picker.

To pass extra arguments to Pi, set `PI_WORKBENCH_PI_ARGS`, for example `--provider <name> --model <id>`. `PI_WORKBENCH_PI` selects a different `pi` executable.

## The changes lens

![Focus on changes, with a removed line shown in place and the change bar](docs/screenshots/lens-focus.png)

There is no diff screen. One setting, **Changes since …** in the status bar, decides what everything is compared against, and every view reads through it:

- **develop** (or `main`, `master`): where your branch left it, so other people's commits on develop never show up as your changes.
- **session start** and **Pi's last turn**: the workspace is snapshotted before each prompt, so you can see exactly what a session, or its latest turn, did.
- **last commit.**

In the explorer, changed files are bright with their line counts and folders show how many changed files they hold; **Changed** collapses the tree to just those files. In an editor, changed lines carry a bar, removed lines appear as red ghost lines where they were, and **Focus** fades everything more than three lines from a change. The bar over the change under the cursor can revert it (undoable), drop it into the composer to ask Pi, or mark it seen so it goes quiet; Alt+Down moves to the next change and on into the next changed file.

The lens reads git without running the repository's filters or hooks, and the workspace is watched, so changes from a terminal or formatter are folded into open files and the lens like Pi's.

![The explorer through the lens](docs/screenshots/lens-explorer.png)

### Who changed it

![Pi's lines, a teammate's commit, your edit, and a reindent filtered out](docs/screenshots/origins.png)

Changed lines carry the colour of whoever wrote them:

- **Pi:** lines Pi wrote, recognised against the edits recorded in Pi's own sessions for this folder, including sessions run from the command line.
- **you:** your edits, and lines you committed.
- **others:** lines committed by someone else, from `git blame` of the last commit compared with your `user.email`.

Changes that only reformat (the same text once whitespace is ignored or, with [difftastic](https://difftastic.wilfred.me.uk) installed, the same syntax tree) keep a hairline in the margin and stay out of the way of navigation; **Show or hide formatting-only changes** in the palette brings them back. With **Changed** on, files are grouped under the Pi turn that changed them, newest first, with each turn's prompt; the rest are under **Other changes**.

## Finding your way

![Search across files](docs/screenshots/search.png)

**Ctrl+P** opens files by name with fuzzy matching. **Ctrl+Shift+F** searches file contents; Enter runs the search and Enter again opens the selected hit at its line. Both skip what `.gitignore` ignores, and both are in the command palette. Files open highlighted and keep their cursor and scroll as you switch tabs; the footer shows the cursor's line and column.

## Shortcuts

Use Ctrl on Windows/Linux or Cmd on macOS. Session cycling uses Ctrl everywhere, because Cmd+Tab switches apps on macOS.

| Shortcut | Action |
| --- | --- |
| Ctrl/Cmd+N | New session |
| Ctrl+Tab, Ctrl+Shift+Tab | Next or previous session |
| Ctrl/Cmd+1 … 9 | Go to session 1 … 9 |
| Ctrl/Cmd+J | Show the conversation |
| Ctrl/Cmd+P | Open file by name |
| Ctrl/Cmd+Shift+F | Search in files |
| Alt+Down, Alt+Up | Next or previous change, across files |
| Ctrl/Cmd+Shift+H | Focus on changes |
| Ctrl/Cmd+Shift+G | Show only changed files |
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
| [session.rs](apps/desktop/src/session.rs) | One Pi session: process, timeline, questions, history, model and usage |
| [sessions.rs](apps/desktop/src/sessions.rs) | Open sessions, the active one, and recent sessions from Pi's store |
| [workspace.rs](apps/desktop/src/workspace.rs) | Opening files, panes, confirmations, layout |
| [documents.rs](apps/desktop/src/documents.rs) | Buffers, saves, reloads, merging outside changes |
| [lens.rs](apps/desktop/src/lens.rs) | The changes lens: baseline, changed files, per-file diffs, seen changes |
| [origins.rs](apps/desktop/src/origins.rs) | Who wrote each changed line, and which changes only reformat |
| [watcher.rs](apps/desktop/src/watcher.rs) | Watching the workspace for outside changes |
| [markdown.rs](apps/desktop/src/markdown.rs) | Reply markdown to timeline blocks |
| [syntax.rs](apps/desktop/src/syntax.rs) | Highlighting open files off the UI thread |
| [explorer.rs](apps/desktop/src/explorer.rs) | Directory loading and cache |
| [commands.rs](apps/desktop/src/commands.rs) | Actions and shortcuts |
| [ui/](apps/desktop/src/ui) | Views and styling: rail, conversation, panes, changes |
| [ui/lens_doc.rs](apps/desktop/src/ui/lens_doc.rs) | Ghost lines, change bars, focus and highlighting inside an editor |
| [ui/finder.rs](apps/desktop/src/ui/finder.rs) | Open by name and search in files |
| [workbench-core](crates/workbench-core/src) | File operations, layout storage, line diffs, merges, read-only git access and blame, difftastic |
| [workbench-text](crates/workbench-text/src) | Syntax highlighting, the gitignore-aware file index and fuzzy matching, content search |
| [pi-client](crates/pi-client/src) | Starting Pi in RPC mode, its events and commands, its session store |

To add an action, update `Command`, `COMMANDS`, `Command::details`, and `execute` in `commands.rs`. Keep its behavior in the module that owns the relevant state.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

### Testing with a fake model

[scripts/fake-model.mjs](scripts/fake-model.mjs) is a scripted OpenAI-compatible server for running Pi without a provider. A prompt that is a JSON array of steps drives it, and `FAKE_MODEL_SCRIPT` names a file of steps to use for any other prompt; see the file header. Point Pi at it with a separate agent directory so your own configuration and sessions are untouched:

```sh
node scripts/fake-model.mjs 47124 &
mkdir -p /tmp/pi-fake && echo '{"providers":{"fake":{"baseUrl":"http://127.0.0.1:47124/v1","api":"openai-completions","apiKey":"fake","models":[{"id":"fake"}]}}}' > /tmp/pi-fake/models.json
PI_CODING_AGENT_DIR=/tmp/pi-fake PI_WORKBENCH_PI_ARGS="--provider fake --model fake" cargo run -p pi-workbench
```

`PI_CLIENT_LIVE=1 cargo test -p pi-client` runs the same setup against the installed `pi`.

## Current limitations

- UTF-8 files only; 4 MiB per file, 64 open documents
- No LSP or terminal yet
- Focus fades unchanged lines rather than folding them; the editor needs equal line heights
- Tab changes reset cursor and scroll position
- Sessions share one checkout; two sessions editing the same lines will conflict
- Workspace layouts are saved on normal close; unsaved edits have no crash recovery
