# DSHDesktop

**中文说明 → [README.zh-CN.md](README.zh-CN.md)**

A desktop shell for [deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) (dsh, DeepSeek's agent harness CLI). It bundles a portable Node.js runtime and the `@deepseek-ai/dsh` package, so the official dsh Web UI runs as a double-clickable Windows app — **no Node.js, no terminal, no setup**.

| Dark | Light |
| --- | --- |
| ![dark](docs/screenshots/main-dark.png) | ![light](docs/screenshots/main-light.png) |

## Features

- **Zero prerequisites** — Node.js 24 and dsh ship inside the installer; works out of the box on Windows 10/11 x64 (WebView2 is auto-installed if missing)
- **One window, official UI** — spawns `dsh web` on a free loopback port and opens the official Web UI once it's ready
- **Tray-resident** — closing the window hides it to the tray; tray menu: open / diagnostics / restart service / quit
- **Native notifications** — dsh approval requests and questions become Windows toasts while the window is hidden
- **Crash resilience** — the dsh process is supervised and restarted with exponential backoff
- **Theme following** — the title bar follows dsh's light/dark/system setting in real time
- **Diagnostics panel** — service state, port, PID, live logs, one-click restart, autostart toggle
- **Single instance** — a second launch just focuses the existing window

## Download & install

Grab `DSHDesktop_<version>_x64-setup.exe` from [Releases](../../releases) and run it. That's it — no administrator rights required (per-user install).

- Installed size: ~242MB (installer ~45MB)
- User data lives in `%LOCALAPPDATA%\DSHDesktop\` (dsh settings, sessions, logs)
- Silent install: `DSHDesktop_<version>_x64-setup.exe /S` (optionally `/D=C:\path\to\dir`)
- To upgrade: quit the app (tray → 退出 / Quit) or uninstall the old version first, then run the new installer

> DSHDesktop is an unofficial community shell. dsh itself is developed by DeepSeek at [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness).

## How it works

```
main window (splash → official dsh Web UI at http://127.0.0.1:<port>)
      │ Tauri IPC
Rust core: runtime / process supervision / tray / notifications / theme / diagnostics
      │ spawn (no console window, DSH_HOME isolated to the app data dir)
bundled node.exe + dsh web --port <free port>   (binds 127.0.0.1 only)
```

dsh events (approval requested, question asked) are consumed over dsh's WebSocket channel `/api/events.mux`; everything platform-specific sits behind a `Platform` trait to leave the door open for macOS/Linux. Full details: **[docs/design.md](docs/design.md)** ([中文](docs/design.zh-CN.md)).

## Build from source

Prerequisites: Rust (stable), Node.js ≥ 22, pnpm 11.

```bash
pnpm install

# prepare the bundled runtime (pick one)
powershell -File scripts/fetch-runtime.ps1        # real runtime (Node 24 + dsh 0.1.0-rc.6)
powershell -File scripts/use-fixture-runtime.ps1  # or a lightweight fake-dsh fixture for shell debugging

pnpm tauri dev
```

Tests and checks:

```bash
cd src-tauri && cargo test     # Rust unit + integration tests (24)
pnpm check && pnpm build       # frontend type-check and build
```

Package the NSIS installer:

```bash
powershell -File scripts/fetch-runtime.ps1
pnpm tauri build               # → src-tauri/target/release/bundle/nsis/DSHDesktop_*_x64-setup.exe
```

End-to-end acceptance on a real install (uninstall → install → launch → full checks → screenshots):

```powershell
powershell -File scripts/acceptance.ps1 -SetupExe src-tauri/target/release/bundle/nsis/DSHDesktop_0.1.0_x64-setup.exe
```

## Project docs

- [Design document (EN)](docs/design.md) / [设计文档（中文）](docs/design.zh-CN.md) — architecture, modules, packaging, testing, known limitations
- [AGENTS.md](AGENTS.md) — contributor guide: layout, commands, pitfalls

## Multi-platform roadmap

Windows x64 only for now. All platform differences are isolated behind `src-tauri/src/platform/` (`Platform` trait); enabling the macOS/Linux rows in the CI matrix requires implementing `platform/{macos,linux}.rs` and teaching `scripts/fetch-runtime.ps1` the new triplets — see the design doc §10.

## License

[MIT](LICENSE). Upstream dsh is also MIT (Copyright 2026 DeepSeek).
