# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.8] - 2026-08-17

### Added

- First-launch window centering: the main window and all on-demand tray windows (diagnostics / settings / skills / MCP / remote) open at screen center when no geometry is remembered; the window-state plugin's restore still overrides the default before the first visible frame, so later launches keep the previous position with no flash
- Configurable notification rules: three notification types — task confirmation (approval requested), choice pending (question requested), reply finished (turn completed) — each with its own enable toggle and timing choice (only in background / always); background means no app window is focused, so toasts never interrupt while you are working in the app

### Changed

- The completion-notification toggle migrated into `notify.turn_done` (existing `notify_on_completion` values carry over automatically); completion sound and preview now belong to the reply-finished rule

## [0.1.7] - 2026-08-17

### Added

- Remote access: one tray click brings the full dsh Web UI to a phone or remote browser via a token-bearing link and QR code. Relay is Cloudflare Quick Tunnel (`cloudflared.exe` bundled in the runtime); zero server, account, or configuration
- Embedded token-gate reverse proxy (`remote/proxy.rs`): `?token=` → 302 + HttpOnly cookie, constant-time compare, fixed 500ms delay on wrong tokens, HTTP streaming forward and WebSocket frame bridging to dsh; browser-marker headers (`origin`/`referer`/`sec-fetch-*`) are stripped so dsh's /api trust fence accepts tunneled requests
- cloudflared supervision with stdout URL parsing, exponential-backoff restarts (new domain on reconnect, token unchanged), and process-tree kill on stop
- Tray submenu (start/stop with mutually exclusive enabled states, copy link, show QR), `#/remote` window with QR SVG, and a remote-access row in the diagnostics panel
- Token hygiene: the token never lands in `events.log` (status lines omit the link, tunnel output is redacted) or in toast bodies

## [0.1.6] - 2026-08-17

### Added

- Theme following extended to the shell's own pages: local pages (diagnostics, skills, MCP, settings, splash) converge on CSS variables and switch with dsh's `ui-theme.preference`; tray menu follows via uxtheme `SetPreferredAppMode`
- Language following: new i18n module reads dsh `locale.preference` (zh/en, default from system UI language); local pages, tray menu, window titles, startup progress, notifications, and command errors are fully bilingual

## [0.1.5] - 2026-08-16

### Added

- Built-in custom sounds for the completion notification (silent/default/im/mail/reminder/sms), with a preview command; bundled via `resources/sounds/*.wav`

## [0.1.4] - 2026-08-16

### Added

- Skills management: enable/disable skills by moving them between `skills/` and `skills-disabled/` (hot-reloaded by dsh's watcher), first-launch seeding, import from codex/claude/opencode with conflict handling, and deletion
- MCP management: read/write dsh's `cordis.patch.yml` MCP client entries (atomic writes, BOM-tolerant), toggle via native `disabled` flag with HMR taking effect without restart, import from claude/codex/opencode configs

## [0.1.2] - 2026-08-16

### Added

- Settings window: customizable zoom step (1-25%) and shortcuts, close-window behavior (hide to tray or quit), completion-notification toggle, first-launch theme seeding
- Splash shows a "takes a few minutes" hint only on first launch

### Fixed

- Remember window size/position across restarts via the window-state plugin

## [0.1.1] - 2026-08-16

### Added

- UI zoom in/out (Ctrl+Shift+= / Ctrl+Shift+-, 2% step), persisted per user

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
