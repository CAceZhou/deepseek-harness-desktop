# DSHDesktop

English | [Chinese](README.zh-CN.md)

A desktop shell for [deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) (dsh), DeepSeek's agent harness CLI. The installer bundles a portable Node.js runtime and the `@deepseek-ai/dsh` package, so the official dsh Web UI runs as a regular Windows application with no prerequisites.

| Dark | Light |
| --- | --- |
| ![dark](docs/screenshots/main-dark.png) | ![light](docs/screenshots/main-light.png) |

## Features

- **Zero prerequisites.** Node.js 24 and dsh ship inside the installer; the app works out of the box on Windows 10/11 x64. WebView2 is installed automatically if missing.
- **Official UI in a native window.** Spawns `dsh web` on a free loopback port and opens the official Web UI as soon as it is ready.
- **Guided first launch.** A stage-based progress bar shows runtime preparation, service startup, and readiness while the app initializes for the first time.
- **Tray resident.** Closing the window hides it to the tray. Tray menu: open, diagnostics, restart service, quit.
- **Native notifications.** dsh approval requests and questions become Windows notifications while the window is hidden.
- **Crash resilience.** The dsh process is supervised and restarted with exponential backoff.
- **Theme following.** The title bar follows dsh's light, dark, or system setting in real time.
- **Diagnostics panel.** Service state, port, PID, live logs, one-click restart, and an autostart toggle.
- **Single instance.** A second launch simply focuses the existing window.

## Download and install

Download `DSHDesktop_<version>_x64-setup.exe` from [Releases](../../releases) and run it. No administrator rights are required; the app installs per user.

- Installed size: about 242 MB (installer about 45 MB)
- User data lives in `%LOCALAPPDATA%\DSHDesktop\` (dsh settings, sessions, and logs)
- Silent install: `DSHDesktop_<version>_x64-setup.exe /S` (add `/D=C:\path\to\dir` to choose the install directory)
- To upgrade, quit the app from the tray menu or uninstall the previous version, then run the new installer

> DSHDesktop is an unofficial community shell. dsh itself is developed by DeepSeek at [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness).

## How it works

```
main window (splash, then the official dsh Web UI at http://127.0.0.1:<port>)
      │ Tauri IPC
Rust core: runtime / process supervision / tray / notifications / theme / diagnostics
      │ spawn (no console window, DSH_HOME isolated under the app data directory)
bundled node.exe + dsh web --port <free port>   (binds 127.0.0.1 only)
```

dsh events (approval requested, question asked) are consumed over dsh's WebSocket channel `/api/events.mux`. Everything platform-specific sits behind a `Platform` trait, leaving the door open for macOS and Linux. Full details in [docs/design.md](docs/design.md).

## Build from source

Prerequisites: Rust (stable), Node.js 22 or later, pnpm 11.

```bash
pnpm install

# prepare the bundled runtime (pick one)
powershell -File scripts/fetch-runtime.ps1        # real runtime (Node 24 + dsh 0.1.0-rc.6)
powershell -File scripts/use-fixture-runtime.ps1  # lightweight fake-dsh fixture for shell debugging

pnpm tauri dev
```

Tests and checks:

```bash
cd src-tauri && cargo test     # Rust unit and integration tests (30)
pnpm check && pnpm build       # frontend type check and build
```

Package the NSIS installer:

```bash
powershell -File scripts/fetch-runtime.ps1
pnpm tauri build               # output: src-tauri/target/release/bundle/nsis/DSHDesktop_*_x64-setup.exe
```

End-to-end acceptance on a real install (uninstall, install, launch, full checks, screenshots):

```powershell
powershell -File scripts/acceptance.ps1 -SetupExe src-tauri/target/release/bundle/nsis/DSHDesktop_0.1.0_x64-setup.exe
```

## Project docs

- [Design document](docs/design.md) (English) and [Chinese version](docs/design.zh-CN.md): architecture, modules, packaging, testing, known limitations
- [AGENTS.md](AGENTS.md): contributor guide with layout, commands, and pitfalls
- [CHANGELOG.md](CHANGELOG.md): release history

## Multi-platform roadmap

Windows x64 only for now. All platform differences are isolated behind `src-tauri/src/platform/` (the `Platform` trait). Enabling the macOS and Linux rows in the CI matrix requires implementing `platform/{macos,linux}.rs` and teaching `scripts/fetch-runtime.ps1` the new triplets. See section 10 of the design document.

## License

[MIT](LICENSE)
