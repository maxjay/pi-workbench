# Workbench

A window on Claude Code or Pi, on Windows, with the agent, your code and your terminals in WSL. Three things in it:

- **The scratchpad**: ask something from Home, with no repo needed.
- **Codespaces**: a folder in WSL with its files, an editor, terminals, and the agent beside them.
- **Pull requests and tickets**: yours on Home, and each one in a tab of its own, through [dova](https://dova.maxjay.dev) 0.4.4 or newer.

**Status:** version 2, written from nothing. All 8 steps of the plan are built: the window, `wbd` in WSL, the look, codespaces with their files and an editor, terminals, the agent (Pi or Claude Code) with the scratchpad, the installer, your pull requests on Home, and creating a pull request from a branch. The design of those is the canvas [Workbench: pull requests first](https://claude.ai/artifact/MZ8HA4JYAe6RkNRzP4NKni).

After the plan, the pull request's tab and the ticket's tab were written again from the canvas [Pull request review](https://claude.ai/artifact/UiMMnaYNVHsSe3XExdGkFP):

- **A pull request's tab** (`app/ui/src/views/pull`). The changed files as a tree, a file's diff in one column or split, and what is said under the lines it is about. A thread is one chat: comments other people see, and what you and the agent say to each other, which only you see. Ask is a switch on the line you write in (Tab): on, Enter goes to the agent; off, Enter posts. Draft has the agent write the comment for you to change and post. A right-click on lines gives Comment and Ask. Review has the agent read the whole change: its summary is in the chat on the right and what it finds is under the lines, for you alone. The overview has the title and description (yours to change, with pictures), the tickets, what must be true to complete, and the threads about the whole of it. Your vote and what becomes of it are two buttons in the bar.
- **A ticket's tab** (`app/ui/src/views/ticket`). What it has depends on its type. The parts of its text, its fields, its states and the type that goes under it are read from Azure DevOps: the type's form, its states and the project's backlog levels. Fields are changed where they are, the tickets under it are added by typing and ticked off in place, and its discussion is the same chat as a thread.
- What you and the agent say to each other, and which files you have read, are kept in the window's own storage. They never go to Azure DevOps.

## How it's built

```
 Windows                                   WSL
┌─────────────────────────────┐          ┌────────────────────────────┐
│ app/ui       the page       │          │ crates/wbd                 │
│  Solid + TypeScript         │          │  Rust, one process: it     │
│            │ ▲              │  stdio   │  does the work the page    │
│            ▼ │              │◄────────►│  asks for, next to the     │
│ app/src-tauri  the app      │  JSON    │  code, the agents and dova │
│  a window, and a pipe to wbd│  lines   │                            │
└─────────────────────────────┘          └────────────────────────────┘
        crates/protocol: every message both sides can send
```

- **The page** draws everything and keeps a copy of what `wbd` last told it.
- **The app** opens the window, starts `wbd` through `wsl.exe`, and carries lines both ways. It doesn't read them.
- **wbd** runs inside WSL and does the work.
- **The protocol** is JSON-RPC 2.0, one object per line. The Rust types in `crates/protocol` are the source, and the TypeScript types in `app/ui/src/protocol` are generated from them by `cargo test -p protocol`.

## Develop

`wbd` is built inside WSL, the page and the app on Windows.

```sh
wsl -e sh -l scripts/wbd.sh        # build wbd and put it in ~/.local/bin, in WSL
cd app/ui && npm install
npm run dev                        # the page in a browser, at 127.0.0.1:5173
npm run tauri dev                  # the app
npm run check                      # Biome and TypeScript
```

In WSL: `cargo test -p protocol -p wbd`.

## Release

Set the version in `Cargo.toml`, then push a tag of the same version, like `v2.0.0-alpha.2`. That builds the app and its installer, publishes both as a GitHub release, and puts them on [workbench.maxjay.dev](https://workbench.maxjay.dev) beside the page in `site/public`: `workbench.exe` is the app as one file that runs as it is, and `workbench-setup.exe` installs it.

## Licence

Copyright 2026 Max Jay. All rights reserved. This is not open source. See [LICENSE](LICENSE).
