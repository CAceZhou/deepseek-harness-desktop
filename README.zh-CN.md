# DSHDesktop

**English → [README.md](README.md)**

[deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（dsh，DeepSeek agent harness CLI）的 Windows 桌面壳应用。安装包内嵌 Node.js 便携运行时与 `@deepseek-ai/dsh`，把 dsh 官方 Web UI 变成双击即用的桌面应用——**不需要装 Node、不需要开终端、零配置**。

| 深色 | 浅色 |
| --- | --- |
| ![深色](docs/screenshots/main-dark.png) | ![浅色](docs/screenshots/main-light.png) |

## 功能

- **零前置依赖** — Node.js 24 与 dsh 随安装包分发，Windows 10/11 x64 装完即用（缺 WebView2 时安装程序自动联网安装）
- **一个窗口，官方界面** — 自动在空闲回环端口拉起 `dsh web`，就绪即打开官方 Web UI
- **托盘常驻** — 关窗最小化到托盘；托盘菜单：打开主界面 / 诊断面板 / 重启服务 / 退出
- **原生通知** — 窗口隐藏时，dsh 的待批准、待回答事件转为 Windows 通知
- **崩溃自愈** — dsh 进程受监督，崩溃后指数退避自动重启
- **主题跟随** — 标题栏实时跟随 dsh 的浅色/深色/跟随系统设置
- **诊断面板** — 服务状态、端口、PID、实时日志、一键重启、开机自启开关
- **单实例** — 重复启动只会聚焦已有窗口

## 下载与安装

从 [Releases](../../releases) 下载 `DSHDesktop_<版本>_x64-setup.exe` 直接运行。无需管理员权限（按用户安装）。

- 安装后约 242MB（安装包约 45MB）
- 用户数据在 `%LOCALAPPDATA%\DSHDesktop\`（dsh 设置、会话、日志）
- 静默安装：`DSHDesktop_<版本>_x64-setup.exe /S`（可加 `/D=目录` 指定安装位置）
- 升级：先退出应用（托盘 → 退出）或卸载旧版，再运行新安装包

> DSHDesktop 是非官方的社区壳应用。dsh 本体由 DeepSeek 在 [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) 开发。

## 工作原理

```
主窗口（启动画面 → 官方 dsh Web UI，http://127.0.0.1:<port>）
      │ Tauri IPC
Rust 核心：运行时 / 进程监督 / 托盘 / 通知 / 主题跟随 / 诊断
      │ spawn（无控制台窗口，DSH_HOME 隔离到应用数据目录）
内嵌 node.exe + dsh web --port <空闲端口>（仅绑 127.0.0.1）
```

dsh 事件（待批准、待回答）通过其 WebSocket 通道 `/api/events.mux` 订阅；平台相关代码全部收敛在 `Platform` trait 后面，为 macOS/Linux 预留。详见 **[设计文档](docs/design.zh-CN.md)**（[English](docs/design.md)）。

## 从源码构建

前置：Rust 稳定版、Node.js ≥ 22、pnpm 11。

```bash
pnpm install

# 准备运行时（二选一）
powershell -File scripts/fetch-runtime.ps1        # 真实运行时（Node 24 + dsh 0.1.0-rc.6）
powershell -File scripts/use-fixture-runtime.ps1  # 或 fake-dsh fixture（轻量，供壳调试）

pnpm tauri dev
```

测试与检查：

```bash
cd src-tauri && cargo test     # Rust 单测 + 集成测试（24 个）
pnpm check && pnpm build       # 前端类型检查与构建
```

打包（NSIS 安装包）：

```bash
powershell -File scripts/fetch-runtime.ps1
pnpm tauri build               # 产物在 src-tauri/target/release/bundle/nsis/
```

真实安装上的端到端验收（卸载 → 安装 → 启动 → 全项校验 → 截图）：

```powershell
powershell -File scripts/acceptance.ps1 -SetupExe src-tauri/target/release/bundle/nsis/DSHDesktop_0.1.0_x64-setup.exe
```

## 项目文档

- [设计文档（中文）](docs/design.zh-CN.md) / [Design (EN)](docs/design.md) — 架构、模块、打包、测试、已知限制
- [AGENTS.md](AGENTS.md) — 贡献指南：目录结构、常用命令、坑

## 多平台路线

当前仅 Windows x64。平台差异都收口在 `src-tauri/src/platform/` 的 `Platform` trait；启用 CI matrix 中 macOS/Linux 行前，需实现 `platform/{macos,linux}.rs` 并让 `scripts/fetch-runtime.ps1` 支持对应 triplet——清单见设计文档 §10。

## License

[MIT](LICENSE)。上游 dsh 同为 MIT（Copyright 2026 DeepSeek）。
