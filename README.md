# Pi Workbench

[![CI](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/maxjay/pi-workbench/actions/workflows/ci.yml)

A Windows workbench for the [Pi](https://pi.dev) coding agent, with Pi, your
code and your terminals in WSL. It works like a browser: Home, codespaces,
scratchpads and tasks are tabs, and Ctrl K goes anywhere.

**Status:** v1, stage 2 of 5 done. Tabs, Home, Ctrl K, Settings for Pi's
instructions (AGENTS.md), MCP servers and models; the scratchpad: Pi
sessions in folders of their own, active or done, with the files Pi writes
beside the conversation, its questions and permission prompts as cards, and
the model and thinking level per conversation; and tasks: prompts Pi runs
on a schedule, with the app in the tray and starting at sign-in so they run
with no window open. Codespaces, sessions in codespaces and subagents come
next.

## How it's built

A Tauri 2 window draws the UI (Solid and TypeScript). Behind it, **wb-agent**,
a small Rust process inside WSL, does the work: it reads and writes Pi's files
and will run Pi's sessions, git and terminals. They talk in JSON lines over
`wsl.exe`. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Install

Download the latest build from
[Releases](https://github.com/maxjay/pi-workbench/releases): the installer
(`…-windows-x64-setup.exe`, installs for your user, no admin rights) or the
portable `…-windows-x64-portable.exe`. You need WSL with a default distro and
Pi installed there. The first start installs wb-agent into `~/.local/bin` in
that distro.

Every CI run also uploads a build, as `pi-workbench-windows`.

## Develop

```sh
cargo test                                   # protocol, wb-agent, and the TS types
cargo build -p wb-agent
cd app/ui && npm install
npm run dev                                  # the UI in a browser, at localhost:5173
npm run tauri dev                            # the app
npm run check                                # Biome and TypeScript
```

## Release

Bump `version` in the root `Cargo.toml`. When that reaches `main`, the
release workflow builds and publishes it. To release from another branch,
push a tag that matches the version (`git tag v1.2.3 && git push origin v1.2.3`)
or run the workflow by hand. A version like `1.0.0-alpha.1` is published as a
pre-release.

## Licence

Copyright 2026 Max Jay. All rights reserved. This is not open source: no
licence to use, copy, change or distribute it is granted without a written
agreement. See [LICENSE](LICENSE).
