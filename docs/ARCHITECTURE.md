# Architecture

Pi Workbench is a Windows app for working with [Pi](https://pi.dev), where
Pi, your code and your terminals live in WSL. It has three parts, and each
does one job.

```
 Windows                                    WSL
┌──────────────────────────────┐          ┌─────────────────────────────┐
│ app/ui      the page         │          │ crates/agent   wb-agent     │
│  Solid + TypeScript          │          │  Rust, one long-lived       │
│  shows state, sends requests │          │  process: owns Pi's files   │
│            │ ▲               │          │  (later: sessions, the      │
│     invoke │ │ events        │          │  scheduler, files, git and  │
│            ▼ │               │  stdio   │  terminals)                 │
│ app/src-tauri  the app       │◄────────►│                             │
│  Tauri 2: a window, and a    │ JSON     │  reads and writes ~/.pi     │
│  pipe to wb-agent            │ lines    │  runs pi                    │
└──────────────────────────────┘          └─────────────────────────────┘
          crates/protocol: every message both sides can send
```

- **The page** (`app/ui`) draws everything and keeps a copy of what wb-agent
  last told it. It never touches the disk.
- **The app** (`app/src-tauri`) opens the window, starts wb-agent through
  `wsl.exe`, and carries lines both ways. It doesn't read them.
- **wb-agent** (`crates/agent`) runs inside WSL, next to Pi and the code.
  It does the work: reading and writing files, and later running Pi
  sessions, git and terminals. Doing that inside WSL avoids the slow
  `\\wsl$` file share entirely.
- **The protocol** (`crates/protocol`) is the one contract between the page
  and wb-agent. The Rust types are the source; the TypeScript types in
  `app/ui/src/protocol` are generated from them.

Why a web page? Tauri uses the WebView2 that ships with Windows, so the app
is a few megabytes and starts quickly. The page is where the hard UI
already exists: a code editor (CodeMirror), a terminal (xterm.js), and
later a view of Pi's Playwright browser. The heavy work happens in Rust in
WSL, not in the page.

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
  `session.close`; `file.read`.
- Each answer has a `kind`, and `ask(call, kind)` in `app/ui/src/agent.ts`
  checks the answer is the kind it expected.
- Error codes are `badRequest`, `io`, `invalid` and `notFound`. A line
  wb-agent can't read still gets an error, with its `id` if it had one.
- A live session sends `{"type":"event","event":"session","id":…,"change":…}`
  whenever it changes: one entry (new, or replacing the one at its index),
  the session's state, the whole conversation after loading it, or that it
  closed. Fast output is gathered up and sent every 40 ms, so a reply
  streaming word by word is about 25 messages a second, not hundreds.
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
    src/question.rs  the question tool's socket; ask_user.ts is the tool
    src/files.rs     files opened for viewing
    tests/stdio.rs   the real binary, spoken to in lines, with a fake pi
app/
  src-tauri/         the app: start wb-agent, carry lines
    src/agent.rs     the child process
    src/launch.rs    which wb-agent to run
    src/wsl.rs       Windows only: run in WSL, install the bundled wb-agent
  ui/
    src/agent.ts     the one connection to wb-agent
    src/protocol/    generated, do not edit
    src/state/       stores: what the page knows, and the calls that change it
    src/components/  pieces used across pages
    src/views/       one file per page; scratch/ for the scratchpad's parts
    dev/             the dev server's bridge to wb-agent
```

## Rules

1. **One contract.** The page and wb-agent agree only through
   `crates/protocol`. Nothing else is shared, and nothing is hand-written
   twice.
2. **wb-agent owns state.** Stores in the page hold what wb-agent last said
   and ask it to change things. They don't compute their own version.
3. **Logic is pure; I/O is at the edges.** `mcp.rs` and `models.rs` turn
   text into values and back. Only `config.rs` and `router.rs` touch the
   disk. Pure functions get unit tests.
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

## Running it

wb-agent and the page, in a browser (any OS, no Tauri needed):

```sh
cargo build -p wb-agent
cd app/ui && npm install && npm run dev     # http://localhost:5173
```

The dev server starts `target/debug/wb-agent` for each page; set `WB_AGENT`
to run another. `PI_CODING_AGENT_DIR` points wb-agent at a different set of
Pi files, which is handy for trying things out.

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
2. **Scratchpad** (done) **and tasks:** Pi sessions in folders of their
   own; then prompts Pi runs on a schedule, which keep running with the
   window closed: the app stays in the tray, holding wb-agent, and starts
   when you sign in.
3. **Codespaces:** a folder in WSL with its files, editor and terminals.
4. **Sessions** inside codespaces.
5. **Subagents.**

Then Pi's browser: Playwright's Chromium in WSL, shown in a tab through
CDP screencasts, which you and Pi both drive.
