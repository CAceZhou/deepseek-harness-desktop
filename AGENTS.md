# DSHDesktop

[deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（dsh，DeepSeek agent harness CLI）的 Windows 桌面壳应用：Tauri 2 窗口内嵌 dsh 官方 Web UI，Node.js 与 dsh 随安装包分发、装完即用。

## 技术栈与形态

- **Tauri 2 + Rust**（`src-tauri/`）：进程监督、运行时部署、托盘、通知、主题跟随、诊断命令
- **Svelte 5 + TypeScript**（`src/`）：仅启动画面（splash）与诊断面板两个本地页面；主界面是导航到的远程 dsh Web UI（`http://127.0.0.1:<port>`）
- 安装包：NSIS（`pnpm tauri build`），单实例、托盘常驻、关窗隐藏到托盘

## 目录结构

```
src-tauri/src/
  lib.rs            Builder 组装：插件(single_instance 必须最先) → setup → 事件桥(dsh-ready→导航)
  platform/         平台抽象 trait（多平台预留）；windows.rs 实现；macos/linux 待实现
  process.rs        DshProcess 监督循环：spawn node bin.js web --port N、指数退避、stop/restart
  runtime.rs        ensure_runtime：安装目录可写则原地运行内嵌运行时；只读则回退部署副本
                    （.version 比对）；原地模式会清理旧版留下的 %LOCALAPPDATA% 部署副本
  notify/           WS 事件源（ws.rs 连 /api/events.mux）+ 过滤器（approval/question requested）
  theme.rs          标题栏主题跟随：轮询 dsh-home/settings.yaml 的 ui-theme.preference
  progress.rs       首启进度模型：阶段权重、百分比映射、结构化 dsh-progress 负载
  tray.rs           系统托盘菜单；diagnostics.rs 状态/日志环形缓冲；commands.rs 7 个 invoke 命令
src/                splash/Splash.svelte、diagnostics/Diagnostics.svelte、App.svelte(hash 路由)
scripts/            fetch-runtime.ps1(下载 Node+dsh+精简)、prune-runtime.ps1(精简运行时)、
                    acceptance.ps1(端到端验收)、shot-window.ps1(窗口截图)、
                    hide-show-theme.ps1(托盘隐藏回归)、get-attr20.ps1(读 DWM 深色属性)、
                    simulate-first-launch.ps1(模拟首启并截图)、
                    use-fixture-runtime.ps1、gen-icon.mjs
docs/design.zh-CN.md / design.md                  设计文档（架构/模块/打包/测试/已知限制，先读它）
```

## 常用命令

```bash
# 开发（需要 fixture 运行时：先跑 scripts/use-fixture-runtime.ps1，再设 DSHDESKTOP_RUNTIME_DIR）
cd src-tauri && cargo test            # 全部测试（30 个：单元+进程集成+WS通知+控制台窗口）
pnpm tauri build                      # 产出 src-tauri/target/release/bundle/nsis/DSHDesktop_*_x64-setup.exe
powershell -File scripts/fetch-runtime.ps1   # 抓取真实运行时到 src-tauri/runtime/windows-x64/
powershell -File scripts/acceptance.ps1 -SetupExe <setup.exe>   # 卸载旧版→安装→启动→全项校验→截图
```

## 关键约定与坑（细节见 docs/design.zh-CN.md）

- **dsh 事实**：Node `^22.19 || >=24`；入口 `lib/bin.js`；`dsh web` 只许绑 127.0.0.1；事件走 **WebSocket** `/api/events.mux`（GET 返回 426），帧格式 `{"type":"server-request","method":"approval/requested",...}`；设置在 `$DSH_HOME/settings.yaml` 的 `ui-theme.preference`（light/dark/system）
- **运行时布局**：暂存 `src-tauri/runtime/<triplet>/`，tauri.conf `resources: ["runtime"]`，安装后 `<install>/runtime/<triplet>/`；`bundle.resources` 相对路径原样映射（`..` 会变 `_up_`，别用）
- **子进程控制台**：`Platform::configure_child_command` 设 CREATE_NO_WINDOW；验收判据是**可见 ConsoleWindowClass 窗口**（conhost 进程存在≠窗口可见）
- **PowerShell 5.1**：含中文的 .ps1 必须 UTF-8 **带 BOM**；别用 PS 改写 `settings.yaml`（会引入 BOM 导致 yaml-rust 解析失败，主题静默回退）
- **Tauri setup 无 tokio 上下文**：spawn_supervised 必须经 `tauri::async_runtime::block_on`
- **Tauri `resource_dir()` 返回 `\\?\` 扩展路径**：Node 加载器不认（EISDIR 崩溃），`runtime::strip_verbatim` 已处理，别绕过 ensure_runtime 自己拼路径
- **外部诊断手段**：`%LOCALAPPDATA%\DSHDesktop\events.log` 记录每个进程事件（1MB 截断），应用卡启动时先看它
- **fixture 用 .cjs**（根 package.json 是 type:module）；`#[tokio::test]` 涉及 std::thread::sleep 时须 `flavor="multi_thread"`
- **NSIS 离线**：github 直连不稳时用 ghproxy.net 预置 `%LOCALAPPDATA%\tauri\NSIS`（含 nsis_tauri_utils.dll，SHA1 须匹配 bundler 常量）
- **托盘 quit 顺序**：先 stop dsh 等 1.5s 再 exit；杀子进程树用 `taskkill /T /F`

## 测试基线

`cargo test` 应全绿（当前 30 个）。`tests/console_window.rs` 的对照组会在屏幕上短暂弹出真实控制台窗口，属正常。改主题/进程/通知逻辑后，跑 `cargo test` + 重装走一遍 `acceptance.ps1`。

## 多平台预留

平台差异都收口在 `platform/mod.rs` 的 `Platform` trait（节点可执行名、运行时目录、triplet、杀进程树、子进程配置、系统深浅色）。CI matrix 里 macos/linux 行已注释，启用前需实现对应 `platform/{macos,linux}.rs` 并在 fetch-runtime 支持对应 triplet。

## 已知限制

- Win10 深色标题栏聚焦时纯黑（系统行为，`DWMWA_CAPTION_COLOR` 仅 Win11）；要做成恒为 dsh 深灰需无边框自绘标题栏——方案要点见 docs/design.zh-CN.md §8，暂缓。
