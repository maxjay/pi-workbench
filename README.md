# Workbench

A window on Claude Code or Pi, on Windows, with the agent, your code and your terminals in WSL. Three things in it:

- **The scratchpad**: ask something from Home, with no repo needed.
- **Workspaces**: a folder in WSL with its files, an editor, terminals, and the agent beside them.
- **Pull requests**: yours on Home, and each one inside its workspace, through [dova](https://dova.maxjay.dev).

**Status:** version 2, written from nothing. Steps 1 to 7 of 8 are done: the window, `wbd` in WSL, the look, workspaces with their files and an editor, terminals, the agent (Pi or Claude Code) with the scratchpad, the installer, your pull requests on Home, and each pull request inside its workspace with its runs and review threads. The design is the canvas [Workbench: pull requests first](https://claude.ai/artifact/MZ8HA4JYAe6RkNRzP4NKni).

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

## Licence

Copyright 2026 Max Jay. All rights reserved. This is not open source. See [LICENSE](LICENSE).
