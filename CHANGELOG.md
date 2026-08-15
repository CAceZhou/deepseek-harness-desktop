# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-15

First public release.

### Added

- Windows x64 desktop shell for `@deepseek-ai/dsh` 0.1.0-rc.6: spawns `dsh web` on a free loopback port and opens the official Web UI when ready
- Bundled portable Node.js 24.19.0 + dsh runtime (zero prerequisites; WebView2 auto-installed by the NSIS installer if missing)
- In-place runtime execution when the install directory is writable, with automatic cleanup of legacy deployed copies; read-only install dirs fall back to a versioned deployed copy under `%LOCALAPPDATA%`
- Process supervision with exponential-backoff restart (up to 5 consecutive failures), one-click restart from tray and diagnostics panel
- Tray residence: close-to-tray, single-instance focus, tray menu (open / diagnostics / restart / quit)
- Native Windows notifications for dsh `approval/requested` and `question/requested` events (via WebSocket `/api/events.mux`), shown only while the window is hidden
- Title-bar theme following of dsh's `ui-theme.preference` (light/dark/system), applied via tao `set_theme` + DWM `DWMWA_USE_IMMERSIVE_DARK_MODE`
- Diagnostics panel: service state, port, PID, 500-line live log ring, autostart toggle
- First-launch progress UI on the splash page: stage-based progress bar with percent and step checklist (shown only when `dsh-home` does not exist yet); real byte-level percent during fallback runtime deployment; ease-toward-95% while waiting for dsh readiness, 100% only on actual ready
- `events.log` debug log at `%LOCALAPPDATA%\DSHDesktop\events.log` (1MB truncation)
- Runtime slimming pipeline (`scripts/prune-runtime.ps1`): installer 45.2MB, installed size 241.8MB
- End-to-end acceptance script (`scripts/acceptance.ps1`) and console-window / process / notification regression tests (24 tests total)

### Known limitations

- On Windows 10 the dark title bar is pure black while focused (system behavior; `DWMWA_CAPTION_COLOR` is Windows 11 only)
- Windows x64 only; macOS/Linux platform hooks are reserved behind the `Platform` trait
