# Pi Workbench

[![CI](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml)

A Windows workbench for the [Pi](https://pi.dev) coding agent, with Pi, your
code and your terminals in WSL. It works like a browser: Home, codespaces,
scratchpads and tasks are tabs, and Ctrl K goes anywhere.

**Status:** v1, stage 1 of 5: the shell. Tabs, Home, Ctrl K, and Settings
for Pi's instructions (AGENTS.md), MCP servers and models. The scratchpad,
tasks, codespaces, sessions and subagents come next.

## How it's built

A Tauri 2 window draws the UI (Solid and TypeScript). Behind it, **wb-agent**,
a small Rust process inside WSL, does the work: it reads and writes Pi's files
and will run Pi's sessions, git and terminals. They talk in JSON lines over
`wsl.exe`. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Try it

Download the Windows build from the latest
[CI run](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml):
`pi-workbench-windows` holds the app and its installer. You need WSL with a
default distro and Pi installed there. The first start installs wb-agent into
`~/.local/bin` in that distro.

## Develop

```sh
cargo test                                   # protocol, wb-agent, and the TS types
cargo build -p wb-agent
cd app/ui && npm install
npm run dev                                  # the UI in a browser, at localhost:5173
npm run tauri dev                            # the app
npm run check                                # Biome and TypeScript
```

## Licence

MIT
