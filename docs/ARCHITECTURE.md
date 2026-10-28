# Architecture

Pi Workbench is a Windows app for working with [Pi](https://pi.dev), where
Pi, your code and your terminals live in WSL. It has three parts, and each
does one job.

```
 Windows                                    WSL
┌──────────────────────────────┐          ┌─────────────────────────────┐
│ app/ui      the page         │          │ crates/agent   wb-agent     │
│  Solid + TypeScript          │          │  Rust, one long-lived       │
│  shows state, sends requests │          │  process: owns Pi's files,  │
│            │ ▲               │          │  sessions, the scheduler,   │
│     invoke │ │ events        │          │  codespaces' files, git and │
│            ▼ │               │  stdio   │  terminals                  │
│ app/src-tauri  the app       │◄────────►│                             │
│  Tauri 2: a window, and a    │ JSON     │  reads and writes ~/.pi     │
│  pipe to wb-agent            │ lines    │  runs pi                    │
└──────────────────────────────┘          └─────────────────────────────┘
          crates/protocol: every message both sides can send
```

- **The page** (`app/ui`) draws everything and keeps a copy of what wb-agent
  last told it. It never touches the disk; the only thing it keeps itself
  is what you've written and not sent, in the window's own storage.
- **The app** (`app/src-tauri`) opens the window, starts wb-agent through
  `wsl.exe`, and carries lines both ways. It doesn't read them.
- **wb-agent** (`crates/agent`) runs inside WSL, next to Pi and the code.
  It does the work: reading and writing files, and running Pi sessions,
  git and terminals. Doing that inside WSL avoids the slow `\\wsl$` file
  share entirely.
- **The protocol** (`crates/protocol`) is the one contract between the page
  and wb-agent. The Rust types are the source; the TypeScript types in
  `app/ui/src/protocol` are generated from them.

Why a web page? Tauri uses the WebView2 that ships with Windows, so the app
is a few megabytes and starts quickly. The page is where the hard UI
already exists: a code editor (CodeMirror), a terminal (xterm.js), and
and web tabs' conversations beside the page. The editor and terminals load
with the first codespace, not when the app starts. The heavy work happens
in Rust in WSL, not in the page.

## The protocol

One JSON object per line, in both directions.

The page asks:

```json
{"id": 7, "method": "mcp.setEnabled", "params": {"name": "grafana", "enabled": false}}
```

wb-agent replies with the same `id`, and either `ok` or `error`:

```json
{"type": "reply", "id": 7, "ok": {"kind": "done"}}
{"type": "reply", "id": 7, "error": {"code": "notFound", "message": "no MCP server called grafana"}}
```

and says things on its own as events:

```json
{"type": "event", "event": "ready", "version": "1.0.0-alpha.1"}
```

- Methods are named `area.action`: `health`; `config.read`, `config.write`;
  `mcp.list`, `mcp.setEnabled`; `models.list`, `models.setDefault`,
  `models.setThinking`; `sessions.list`, `sessions.setDone`;
  `session.start`, `session.open`, `session.read`, `session.get`,
  `session.send`, `session.unqueue`, `session.abort`, `session.answer`,
  `session.models`, `session.setModel`, `session.setThinking`,
  `session.commands`, `session.close`; `tasks.list`, `tasks.save`,
  `tasks.setPaused`, `tasks.pauseAll`, `tasks.run`, `tasks.stop`,
  `tasks.scheduleFrom`; `files.list`, `files.stat`, `file.read`,
  `file.write`; `codespaces.list`, `codespaces.open`,
  `codespaces.setPinned`, `codespaces.forget`; `folders.list`;
  `sessions.in`; `git.status`, `git.original`; `terminals.list`,
  `terminal.open`, `terminal.read`, `terminal.input`, `terminal.resize`,
  `terminal.close`.
- Each answer has a `kind`, and `ask(call, kind)` in `app/ui/src/agent.ts`
  checks the answer is the kind it expected.
- Error codes are `badRequest`, `io`, `invalid`, `notFound`, `pi` and
  `conflict` (a save refused because the file changed since it was read).
  A line wb-agent can't read still gets an error, with its `id` if it had
  one.
- Requests are answered as they can be, so a slow one doesn't hold up the
  rest; replies can come in any order. Keys typed in a terminal are the
  exception, handled in the order they came.
- A live session sends `{"type":"event","event":"session","id":…,"change":…}`
  whenever it changes: one entry (new, or replacing the one at its index),
  the session's state, the whole conversation after loading it, or that it
  closed. Fast output is gathered up and sent every 40 ms, so a reply
  streaming word by word is about 25 messages a second, not hundreds.
- Whenever a task or one of its runs changes, wb-agent sends every task
  as `{"type":"event","event":"tasks","list":…}`.
- A terminal's output comes as `{"type":"event","event":"terminal","id":…,"data":…}`,
  as it printed it, every 30 ms at most; and every terminal, as
  `terminals`, whenever one opens, closes, or starts or finishes running
  something.
- `cargo test -p protocol` writes the TypeScript types. CI fails if they
  differ from what's committed.

To add a method: add a variant to `Call` (and to `Answer` if it returns
something new) in `crates/protocol`, handle it in `crates/agent/src/router.rs`,
run `cargo test`, and call it from a store in `app/ui/src/state`.

## Sessions

wb-agent runs each session as `pi --mode rpc` in its folder, so Pi uses its
own settings, MCP servers, extensions and session store, exactly as `pi` in
a terminal does. Scratchpads each get a folder under
`~/.local/share/pi-workbench/scratchpads`.

- `pi.rs` reads Pi's lines into events and writes its commands
  (Pi's docs/rpc.md). A command wb-agent needs the answer to goes with an
  id, and `Live::request` waits for Pi's reply to that id.
- `transcript.rs` applies those events to the session's state and entries,
  and remembers what changed. It's pure, so it's tested line by line.
- `session.rs` owns the process: a task reads Pi's output into the
  transcript and sends the changes.
- No Pi runs until you start a scratchpad or write in one, and wb-agent
  stops a session's Pi once it has been idle for 30 minutes
  (`WB_IDLE_SECS`): not working, not waiting on an answer, nothing queued,
  nothing said. A run that takes hours is never idle. Writing in it again
  starts Pi again.
- `store.rs` lists saved sessions from Pi's own store, and reads one's
  current branch so a saved scratchpad shows without starting Pi. wb-agent
  keeps no copy of any conversation: writing in a saved one runs
  `pi --session <file>`.
- `done.rs` keeps which scratchpads are marked done, as Pi has no such
  idea: a list of session files in `done.json`. Marking one done stops its
  Pi; writing in it again brings it back.
- `question.rs` and `ask_user.ts` are the workbench's own question tool.
  Pi's question tools draw in its terminal, which RPC mode doesn't have, so
  wb-agent loads this extension into every Pi it starts (`--extension`).
  The tool writes its questions as one line to the session's Unix socket,
  named in `WB_ASK`, and waits for the answers on one line back; wb-agent
  shows them as the session's `ask` meanwhile. Extensions' own dialogs
  (`select`, `confirm`, `input`, `editor`, as Pi's permission gate uses)
  come through RPC as usual.

wb-agent's own files (`done.json`, the extension) live in
`~/.local/share/pi-workbench`, or `WB_DATA_DIR`; scratchpad folders in
`scratchpads` there, or `WB_SCRATCH_DIR`.

## Tasks

A task is a prompt Pi runs at set times, every few hours, or only when you
run it. `tasks.rs` keeps them and their runs in `tasks.json`, in wb-agent's
folder, and gives each a folder of its own to work in under `tasks/`.

- Every 20 seconds it looks for runs that are due, in local time
  (`schedule.rs`, with jiff). A run whose time passed while wb-agent wasn't
  running, or while tasks were paused, runs once, late, however many it
  missed. One due while another run is going is skipped.
- A run is a prompt in a Pi session: the task's one session if it
  continues one, or a new one each run. wb-agent follows it until it ends:
  running, waiting on your answer, done (with a line from Pi's reply and
  the files it wrote), failed or stopped. Each run knows where it starts in
  its session's conversation, so the page can show where each begins.
- Tasks only run while wb-agent does, so the app keeps it running: closing
  the window keeps the app in the tray, it starts in the tray when you sign
  in, and only one copy runs (`app/src-tauri/src/background.rs`). The page
  tells the tray how many tasks wait on you; the tray's Pause all goes back
  through the page.
- Windows notifications aren't sent yet: the page highlights a run that
  needs you or failed, and `notify_run` is where they'll go.

## Codespaces

A codespace is a folder in WSL opened in the workbench. `codespaces.rs`
keeps the list in `codespaces.json`; nothing is written into the folder.

- **Files and git.** The tree is `files.list`, with git's marks from
  `git.status`, which runs `git status` and `git diff --numstat` in the
  folder (`git.rs`). The editor (CodeMirror) marks what changed since the
  last commit, against the file as `git show HEAD:` has it. The page asks
  again every few seconds while the tab is open, and when Pi finishes a
  tool there.
- **Saving.** `file.write` takes the text the editor last read; if the
  file holds anything else, something changed it since, and the save is
  refused as a `conflict`. Open files are checked for changes on disk
  (`files.stat`) every second and a half: one with nothing unsaved just
  shows the new text, and one with unsaved changes asks which to keep.
  Saves go through a temporary file and keep the file's permissions.
- **Terminals.** `terminals.rs` runs your shell in a pseudo-terminal in
  the folder, sends what it prints as events and keeps the last 256 KB of
  it, so a terminal can be drawn again. What runs in one is whatever leads
  its foreground process group. Closing one hangs up on everything in it.
- **Pi in the terminals.** Sessions that run in a codespace also load
  `terminal.ts`, a tool that lists the codespace's terminals, reads one,
  runs a command in one that's at its prompt, or opens one. It asks
  wb-agent over one socket, named in `WB_TERMINALS`, with the folder Pi
  runs in, so a session only sees its own codespace's terminals.
- **Sessions** are Pi sessions in the codespace's folder, listed with
  `sessions.in`. A task can run in a codespace too, instead of its own
  folder.
- **Language servers** such as jvmd aren't connected yet. They'll plug into
  the editor's bar, with what they can run, and into Ctrl P's `@` symbols:
  `app/ui/src/views/space/integrations.ts` is where.
- Commands wb-agent runs for itself (git, `pi --version`) get a stdin of
  their own: its stdin is the workbench's, and a child that changed it
  could stop requests being read.

## Web tabs

Web tabs are pages in the workbench's own Chromium: wb-browser
(`crates/browser`), a CEF program beside the app, with your profile brought
in from the Chromium you use. The app lays each tab's page over the window
where the page says (`browser_place`); the rest is the page's, as for every
other tab.

- **wb-browser makes the tabs.** Each is a Chrome-style browser (so
  extensions run) drawn as a child of the workbench's window, hidden until
  placed. The app starts it with the window's handle and talks to it in
  lines: what to open and where each tab goes, in; what opened, closed or
  was pressed, out. DevTools' own `Target.createTarget` would open a Chrome
  window, so wb-agent asks wb-browser for a tab instead (`browser/host.rs`,
  through the app), and the gate does the same for Pi's. Shortcuts that are
  the workbench's (Ctrl K, L, T, N, W and comma) come back to the page even
  while a tab has the keys.
- **Attached to once a page.** In CEF a popup that DevTools attaches to as
  it's made never loads, so nothing auto-attaches: wb-agent attaches to
  each page as it's announced, and the gate does Pi's `setAutoAttach` for
  it the same way, answering Pi as the browser would.

- **wb-agent follows the browser** (`browser/`) over the DevTools protocol:
  every page, its title, loading and back/forward, downloads, and history,
  kept in `history.jsonl` for the address bar's suggestions. It drives your
  tabs (`browser.navigate`, `.go`, `.reload`, `.close`) and takes a page's
  picture (`browser.shot`).
- **Reaching it from WSL.** wb-agent runs in WSL and the browser on
  Windows. In the app, wb-agent runs with `--browser-tunnel`: each stream
  it opens to the DevTools port is lines through the app, which connects on
  Windows (`app/src-tauri/src/browser.rs`). WSL's networking never matters.
  In development, `WB_BROWSER=host:port` points wb-agent at any Chromium
  directly.
- **Pi uses the browser** through a Playwright MCP server called `browser`,
  which wb-agent adds to `mcp.json` once. It connects to wb-agent's gate
  (`browser/gate.rs`) on 127.0.0.1, in WSL, with the session in the path
  (`WB_SESSION` from Pi's environment, or `terminal`). Through the gate,
  each Pi sees only its own pages and those its session sits beside, and
  starts on a blank page of its own, never one of yours. Any other
  Playwright server keeps its own browser.
- **Pi's pages stay out of your strip.** A call that used the browser gets
  the page it left Pi on (`Tool.browser`), shown as a card with the page's
  picture; opening it puts the page in your strip, with that session
  beside it. A page Pi opens from a terminal joins your strip at once,
  having no card to open it from.
- **Your pages** get a conversation when you first ask about one: a
  session in a scratchpad folder, kept out of the Scratchpad's list.
- In a plain browser (`npm run dev`) the page area shows the page's
  picture, a moment behind, as there's no Chromium view to lay over it.

## Layout

```
crates/
  protocol/          the messages, and tests that pin their JSON
  agent/             wb-agent
    src/main.rs      read lines, route them, write replies through one writer
    src/router.rs    each call to the function that answers it
    src/config.rs    where Pi's files are; read and write them safely
    src/mcp.rs       what mcp.json means, as text in and text out
    src/models.rs    what models.json and settings.json mean
    src/pi.rs        Pi's RPC lines, in and out
    src/messages.rs  Pi's messages as conversation entries
    src/transcript.rs  a session's state and entries, from Pi's events
    src/session.rs   one live session: Pi's process and its transcript
    src/sessions.rs  the live sessions; starting, resuming, listing
    src/store.rs     Pi's saved sessions
    src/done.rs      which scratchpads are done
    src/tasks.rs     tasks: kept, due, run and followed
    src/schedule.rs  when a schedule is due, in local time
    src/question.rs  the question tool's socket; ask_user.ts is the tool
    src/files.rs     files opened for viewing, saved, and listed for `@`
    src/codespaces.rs  the codespaces, and folders to open as one
    src/git.rs       where a repository stands; a file as the last commit has it
    src/terminals.rs  shells in pseudo-terminals; terminal.ts is Pi's tool for them
    src/browser/     the browser: following it (mod.rs), the DevTools client
                     (cdp.rs, ws.rs), reaching it (dial.rs), history, Pi's
                     server in mcp.json (server.rs) and the gate Pi uses
    tests/stdio.rs   the real binary, spoken to in lines, with a fake pi
app/
  src-tauri/         the app: start wb-agent, carry lines
    src/agent.rs     the child process
    src/background.rs  the tray, closing to it, starting at sign-in
    src/browser.rs   wb-browser, and wb-agent's streams to it
crates/browser/      wb-browser: the CEF host for web tabs
    src/launch.rs    which wb-agent to run
    src/wsl.rs       Windows only: run in WSL, install the bundled wb-agent
  ui/
    src/agent.ts     the one connection to wb-agent
    src/protocol/    generated, do not edit
    src/state/       stores: what the page knows, and the calls that change it
    src/components/  pieces used across pages
    src/views/       one file per page; chat/ for the parts every conversation
                     has; scratch/, task/ and space/ for each page's own
    dev/             the dev server's bridge to wb-agent
```

## Rules

1. **One contract.** The page and wb-agent agree only through
   `crates/protocol`. Nothing else is shared, and nothing is hand-written
   twice.
2. **wb-agent owns state.** Stores in the page hold what wb-agent last said
   and ask it to change things. They don't compute their own version.
3. **Logic is pure; I/O is at the edges.** `mcp.rs` and `models.rs` turn
   text into values and back. Reading and writing happen at the edges:
   `config.rs`, `files.rs` and the modules that keep wb-agent's own files. Pure functions get unit tests.
4. **Keep Pi's files Pi's.** wb-agent edits the files Pi reads, keeps keys
   it doesn't know about and the order they're in, and writes through a
   temporary file so a crash never leaves half a file.
5. **Errors are typed and always shown.** Rust errors are `thiserror`
   enums that become a `Failure`; the page shows every failure where it
   happened. Nothing fails silently.
6. **Small files, few dependencies.** A file does one thing. Add a
   dependency only when writing it ourselves would be worse.
7. **CI is the gate.** Formatting, lints, tests, generated types and both
   builds pass before anything merges.
8. **One set of chat parts.** Every conversation with Pi, in the scratchpad,
   a task, a codespace's sessions or beside any other tab, is drawn with the
   same parts: its entries, the question card that takes the place of the
   box you write in, messages waiting to be sent, the box itself with its
   `@` and `/` picker and model menu, and the status line. A place adds
   what's its own around them (a run's header, a subagent's card) rather
   than drawing its own version. The parts nest: a subagent's card holds
   its own entries, box and questions, and a question takes the place of
   the box of whatever waits on it, so a subagent asking never takes the
   session's box from you.

## Running it

wb-agent and the page, in a browser (any OS, no Tauri needed):

```sh
cargo build -p wb-agent
cd app/ui && npm install && npm run dev     # http://localhost:5173
```

The dev server starts `target/debug/wb-agent` for each page; set `WB_AGENT`
to run another. `PI_CODING_AGENT_DIR` points wb-agent at a different set of
Pi files, which is handy for trying things out. Web tabs need a Chromium:
start one with `--remote-debugging-port=9222 --user-data-dir=<a folder>`
and set `WB_BROWSER=127.0.0.1:9222`.

`cargo build -p wb-browser` fetches CEF's files, or takes them from
`CEF_PATH`; they go next to `wb-browser`, which the app runs from beside
itself. `WB_BROWSER_EXTENSIONS` loads unpacked extensions, comma-separated.

The app:

```sh
cd app/ui && npm run tauri dev
```

On Windows the app runs `~/.local/bin/wb-agent` in the default WSL distro,
installing the copy it carries when that one is missing or a different
version. Set `WB_AGENT` to a path inside WSL to run your own build instead.
Elsewhere it runs the `wb-agent` next to the app, or the one on `PATH`.

## Stages

1. **Browser shell:** tabs, Home, Ctrl K, Settings for Pi's instructions,
   MCP servers and models.
2. **Scratchpad and tasks** (done): Pi sessions in folders of their own;
   prompts Pi runs on a schedule, which keep running with the window
   closed: the app stays in the tray, holding wb-agent, and starts when you
   sign in.
3. **Codespaces** (done): a folder in WSL with its files, editor and
   terminals.
4. **Sessions** inside codespaces (done).
5. **Subagents.**

Web tabs (in progress): working, with wb-browser built by hand. Next:
building and carrying wb-browser and CEF's files with the app on Windows.
