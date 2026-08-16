# DSHDesktop 设计文档

> 面向后续开发者。读完本文应能回答：每个模块为什么存在、改动某处会影响谁、新增平台/功能该从哪里下手。
> 英文版：[design.md](design.md)。

## 1. 项目定位

DSHDesktop 是 [deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（dsh，DeepSeek 的 agent harness CLI，npm 包 `@deepseek-ai/dsh`）的 **Windows 桌面壳应用**。

dsh 本身提供 `dsh web` 命令：在本机 127.0.0.1 上启动一个 Web UI 服务。但它要求用户先装 Node.js 22+、再全局装 dsh、再开终端敲命令——对普通用户门槛太高。DSHDesktop 把这一切打包成双击即用的桌面应用：

- 安装包内嵌 Node.js 便携运行时与 dsh（含全部生产依赖），**用户机器上零前置依赖**（仅需 Windows 10/11 x64，WebView2 缺失时安装程序自动联网安装）；
- 启动后自动拉起 `dsh web`，就绪即在应用窗口中打开官方 Web UI；
- 托盘常驻、关窗最小化到托盘、崩溃自动重启、事件转原生通知、标题栏主题跟随 dsh 设置。

**非目标**：不重写 dsh 的 UI（壳内直接加载官方 Web UI，dsh 升级即 UI 升级）；不修改 dsh 源码（只通过进程参数、环境变量、其暴露的 HTTP/WS 接口交互）；不做安装器之外的系统级集成（不写注册表自启以外的系统项）。

## 2. 关键设计决策

| 决策 | 理由 | 代价 |
| --- | --- | --- |
| 壳内嵌官方 Web UI，不自绘界面 | dsh 处于开发者预览期，UI 迭代快；壳跟随 npm 包版本即可 | 主窗口内容依赖 dsh 进程健康；需要进程监督 |
| 内嵌 Node + dsh 随安装包分发 | 零前置依赖，装完即用 | 安装包 45MB、安装后约 242MB |
| 运行时**原地运行**（安装目录可写时） | 省掉约 230MB 部署副本；NSIS 默认按用户安装即可写 | 需处理只读安装目录回退与旧副本清理 |
| 事件通知走 WebSocket `/api/events.mux` | dsh 官方事件下行通道 | 上游接口不稳定，需适配层隔离 |
| 主题用 2s 轮询 settings.yaml | 文件极小、改动极少；比 inotify 简单且跨平台无差异 | 主题切换最多 2s 延迟 |
| 平台差异全部收口 `Platform` trait | 为 macOS/Linux 预留；`compile_error!` 强制新平台显式实现 | trait 方法需精心设计（见 §10） |

## 3. 总体架构

```
┌────────────────────────── 主窗口 (label: "main") ──────────────────────────┐
│  本地启动画面 (Svelte)  ──dsh 就绪──▶  navigate 到 http://127.0.0.1:<port> │
│  （dsh 官方 Web UI，壳不干预其内部）                                        │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ Tauri IPC（invoke 命令 / emit 事件）
┌──────────────────────────────────┴───────────────────────────────────────┐
│ Rust 核心（src-tauri/src/）                                               │
│  lib.rs      组装：插件 → setup → 事件桥（dsh 状态 → 前端事件/窗口导航）   │
│  runtime.rs  运行时定位：原地运行 / 只读回退部署 / 旧副本清理               │
│  process.rs  DshProcess 监督循环：spawn、就绪探测、指数退避重启             │
│  notify/     WS 订阅 dsh 事件 → 过滤 → 原生通知（窗口隐藏时）              │
│  theme.rs    轮询 dsh 主题设置 → DWM 标题栏着色                            │
│  progress.rs 首启进度模型：阶段权重、百分比映射、结构化事件负载             │
│  tray.rs     托盘菜单；diagnostics.rs 状态/日志；commands.rs 7 个命令      │
│  zoom.rs     UI 缩放：±2% 步进、快捷键钩子注入、ui-zoom.txt 持久化         │
│  platform/   Platform trait（windows.rs 实现；macos/linux 为编译期占位）   │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ spawn（CREATE_NO_WINDOW，DSH_HOME 隔离）
┌──────────────────────────────────┴───────────────────────────────────────┐
│ 内嵌运行时：<install>/runtime/windows-x64/                                 │
│   node.exe + dsh/node_modules/@deepseek-ai/dsh/lib/bin.js                 │
│ 运行方式：node bin.js web --port <空闲端口>（仅绑 127.0.0.1）              │
└──────────────────────────────────────────────────────────────────────────┘
```

数据目录：`%LOCALAPPDATA%\DSHDesktop\`

```
DSHDesktop\
  dsh-home\          dsh 的 DSH_HOME（settings.yaml、sessions、用户数据都在这里）
  events.log         进程事件调试日志（1MB 截断，诊断的最后手段）
  ui-zoom.txt        UI 缩放比例持久化（缺失/损坏回退 100%）
  runtime\           仅"只读安装目录"回退模式下存在（部署副本，带 .version 标记）
```

## 4. 启动时序

1. **插件初始化**：`single_instance` 必须最先注册——第二次启动时聚焦已有主窗口，不重复拉起。
2. **setup**：建托盘 → 注册 `BootstrapInfo`（启动错误兜底）→ 取 `resource_dir` 定位内嵌运行时。
3. **首启判定**：`dsh-home` 在 ensure_runtime 之前不存在即首启（前端据此决定显示进度条还是纯文字）。
4. **ensure_runtime**（§5）：失败不退出，错误写入 `BootstrapInfo` 并 emit `dsh-progress`（stage=error），窗口停在启动画面显示错误（前端会主动 `get_bootstrap_error` 查询，因为错误事件可能早于前端 listen 注册而丢失）。
5. **spawn_theme_follower**：立即按系统主题给标题栏着色一次，然后 2s 轮询。
6. **spawn_supervised**：经 `tauri::async_runtime::block_on` 调用（setup 里没有 tokio 上下文，内部 `tokio::spawn` 依赖它）。状态机：`Starting → Ready{port}`。
7. **事件桥 bridge_event**（lib.rs）：所有 `dsh-progress` 事件都是结构化负载 `{stage, message, percent}`（progress.rs），百分比由后端按阶段权重计算：
   - `runtime`：原地运行 → 0→15%；回退部署 → 按复制字节实时报 0→70%（节流：百分比变化才发）
   - `starting`：15% 或 70%（取决于是否部署过）
   - `ready` → 100% → 写入 port 的 watch 通道（通知 WS 订阅器换端口）→ emit `dsh-ready` → **主窗口 navigate 到 `http://127.0.0.1:<port>/`**
   - `error` → 主窗口 navigate 回本地启动画面
   - 每个事件同时追加到 `events.log`。
8. dsh 就绪后，WS 订阅器连上 `/api/events.mux`，进入稳态。

## 5. 运行时管理

**背景**：安装包把运行时原样放在 `<install>\runtime\windows-x64\`（tauri.conf `resources: ["runtime"]` 做相对路径映射）。早期设计是首次启动时把整个运行时复制到 `%LOCALAPPDATA%`（防只读安装目录），代价是安装后体积翻倍（约 +230MB）。

**现状：两级策略**

```
ensure_runtime(source_dir, app_version):
  1. strip_verbatim(source_dir)     # 剥 \\?\ 前缀（见下方坑）
  2. validate_source                # node.exe 与 dsh bin.js 必须存在，否则报 Incomplete
  3. home = %LOCALAPPDATA%\DSHDesktop\dsh-home（不存在则创建）
  4. 安装目录可写？（create_new 探测文件）
     ├─ 可写 → 清理旧版部署副本（%LOCALAPPDATA%\...\runtime 且带 .version 标记才删）
     │        → 原地运行：node_exe/bin.js 直接指向安装目录
     └─ 只读 → 部署到 %LOCALAPPDATA%\...\runtime：
              .version == app_version 则跳过，否则全量重复制
```

- **工作目录与运行时解耦**：dsh 的 cwd 永远是 `%LOCALAPPDATA%\DSHDesktop`（可写），即使原地运行模式也不污染安装目录。
- **旧副本清理只认 `.version` 标记**：那是我们自己写下的文件，避免误删用户数据。

**坑（已修复，勿回退）**：Tauri 的 `app.path().resource_dir()` 在 Windows 上返回带 `\\?\` 扩展前缀的路径（如 `\\?\F:\DSHDesktop\...`）。Node 的模块加载器不认这种路径——它把 `\\?\F:` 的首段当成盘符相对路径，入口解析直接 `EISDIR: illegal operation on a directory, lstat 'F:'` 崩溃。`strip_verbatim` 只剥"剩余部分是盘符绝对路径"的形态（`\\?\UNC\...` 保留），与 `dunce::simplified` 的保守策略一致。**任何给 Node 的路径都必须经过 ensure_runtime，不要在别处自己拼。**

## 6. 进程监督

`DshProcess::spawn_supervised` 拉起一个 tokio 监督循环，状态机：

```
Starting ──wait_ready 60s 内拿到 HTTP 响应──▶ Ready{port}
   │                                           │ child.wait() 返回（崩溃/退出）
   │ 超时/连续失败 5 次                          ▼
   ▼                                     指数退避 500ms×2（封顶 30s）→ 回到 Starting
Failed（不再自动重启，前端/托盘可手动 restart）
```

- **spawn 参数**：`node bin.js web --port <port>`，`env DSH_HOME=<home>`，`cwd=%LOCALAPPDATA%\DSHDesktop`，stdout/stderr 管道泵入 `LogRing` + 事件流，`kill_on_drop(true)`，再经 `Platform::configure_child_command` 加 `CREATE_NO_WINDOW`（不弹控制台窗口）。
- **端口**：`free_port()` 让 OS 分配空闲端口——返回到使用之间存在竞态窗口，靠"就绪超时即杀、换端口重试"兜底；`wait_ready` 轮询 `http://127.0.0.1:<port>/` 直到拿到**任意** HTTP 响应（不要求 200）。
- **stop/restart**：两个 `tokio::sync::Notify`。stop 置 shutdown 标志并通知，循环杀掉进程树（`taskkill /T /F`，dsh 可能派生 python 等子孙）后进入 `Stopped`；restart 在循环存活时通知其立即重来，循环已退出（Failed/Stopped）时重新 spawn 一个监督循环。
- **tokio 陷阱**：`Child::kill()` 返回 future，不 await 就不执行；泄漏的子进程若继承了 stdout 管道，外层等管道 EOF 会永远阻塞（集成测试曾因此假挂起）。所有子路径都必须 `kill_on_drop` + 显式 `child.wait().await` + 测试里 stdio 全 null。

## 7. 事件通知

```
dsh WS /api/events.mux ──▶ WsSource ──▶ EventFilter ──▶ summarize ──▶ NotifySink
（断线 5s 重连；端口经        （regex 匹配 method）  （提取可读摘要）  （主窗口隐藏时
 watch 通道跟随重启换端口）                                              才弹原生通知）
```

- dsh 的事件帧：`{"type":"server-request","method":"approval/requested","payload":{...}}`。GET 该端点返回 426（仅 WS）。
- 只放行需要用户关注的两类：`approval/requested`（待批准）、`question/requested`（待回答）。
- dsh 的浏览器信任栅栏允许 loopback + 无 Origin 的请求，Rust 客户端天然满足。
- **适配层是有意为之**：`NotifySource` trait 隔离上游不稳定的接口，将来可加 `FileWatchSource`（解析 session jsonl）等替代实现。
- sink 只在主窗口隐藏（托盘态）时弹通知，避免打扰正在操作的用户。

## 8. 主题跟随

目标：dsh 设置里的 `ui-theme.preference`（light/dark/system，存于 `$DSH_HOME/settings.yaml`）变化时，应用所有窗口的标题栏跟着变。

- **2s 轮询**配置文件（文件极小，轮询比 inotify 简单且跨平台无差异）；`system` 经注册表 `AppsUseLightTheme` 解析。
- **BOM 陷阱（已修复，勿回退）**：PowerShell 5.1 的 `Set-Content -Encoding utf8` 会写入 UTF-8 BOM，而 yaml-rust 不接受 BOM——解析失败会**静默回退 system 主题**，表象是"标题栏永远白色"。`read_theme_preference` 先剥 BOM 再解析。教训：**不要用 PowerShell 改写 settings.yaml**。
- **Windows 双管齐下**：
  1. `window.set_theme()` 同步 tao 内部主题状态——不同步的话，tao 可能在窗口事件后用缓存的旧状态覆盖可视效果。隐藏窗口上调用可能报错甚至 panic，必须 `catch_unwind` 兜住；
  2. 直接对 HWND 调 `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE=20)`——无缓存、幂等、对隐藏窗口同样生效，是标题栏颜色的权威来源。attr 20 失败（E_INVALIDARG）时回退旧值 19（Win10 20H1 之前）。
- **已知限制**：Win10 上深色标题栏**聚焦时纯黑、失焦时深灰**是系统行为；`DWMWA_CAPTION_COLOR`(35)/`DWMWA_TEXT_COLOR`(36) 仅 Win11 可用。要做到恒为 dsh 的深灰（#1B1B1C），需要无边框窗口 + `initialization_script` 注入自绘标题栏——暂缓。

## 9. 前端与窗口管理

壳的本地页面只有两个，用 **hash 路由**（`App.svelte` 监听 `hashchange`）：

- `#/`（默认）**Splash.svelte**：启动画面。onMount 先 `invoke('get_bootstrap_error')` 主动查引导错误、`invoke('is_first_launch')` 查首启标记，再 listen 结构化的 `dsh-progress`。**首启时**显示分阶段进度条（百分比数字 + 阶段清单 ✓/●/○）与"首次启动需要部署运行时，可能要花几分钟"提示（仅此分支渲染，后续启动不出现）：runtime/starting 阶段百分比由后端给下限，`starting` 期间前端向 95% 渐近缓动（dsh 无细分进度信号，缓动只是呈现层，永不触顶），`ready` 到 100%。**非首启**维持纯文字 + 不确定滚动条。dsh 就绪后由 **Rust 侧**把主窗口 navigate 到 dsh UI——前端不自己跳。
- `#/diagnostics` **Diagnostics.svelte**：诊断面板（状态/端口/PID/版本、500 行实时日志回填 + `dsh-log` 事件流、重启按钮、开机自启开关）。

窗口行为：

- 主窗口 `main`：**关窗 = 隐藏到托盘**（`CloseRequested` 时 `prevent_close` + `hide`）；托盘"打开主界面"或二次启动（单实例插件）时 `show` + `unminimize` + `set_focus`。
- 诊断窗口 `diagnostics`：托盘菜单按需创建，**关窗 = 销毁**，下次再建。
- 托盘"退出"：先 `stop()` dsh，等 1.5s 让监督循环杀完进程树，再 `exit(0)`。
- 导航到远程 URL 后窗口标题被 dsh 的 `document.title` 覆盖——**外部脚本不要按标题找窗口**（按 PID + 类名，见 `scripts/shot-window.ps1`）。

IPC 命令：commands.rs 7 个——`get_status` / `restart_dsh` / `get_recent_logs` / `get_autostart` / `set_autostart` / `get_bootstrap_error` / `is_first_launch`；另有 zoom.rs 的 `zoom_ui`（见下）。

UI 缩放（zoom.rs）：

- **快捷键**：`Ctrl+Shift+=` 放大、`Ctrl+Shift+-` 缩小，加性步进 ±2 个百分点（clamp 到 25%–500%）。钩子脚本（`HOOK_JS`）由 `on_page_load` 在每次整页加载完成后 eval 注入（`__dshZoomHook` 标志保证幂等，本地 splash 与远程 dsh UI 通用），capture 阶段拦截并 invoke `zoom_ui` 命令，经 WebView2 原生 `SetZoomFactor` 生效——与浏览器 Ctrl++ 同一机制。匹配主用 `e.code` 物理键位，`e.key` 兜底（`'+'`/`'='`/`'-'`/`'_'`）——合成按键与 RDP 注入的 keydown `e.code` 为空，纯 code 匹配会整组失效。
- **持久化**：每次变更即写 `%LOCALAPPDATA%\DSHDesktop\ui-zoom.txt`；缺失/损坏回退 100%；每次页面加载时 `on_page_load` 统一重应用当前缩放（兼作 WebView2 重建后的兜底）。
- **托盘菜单**：放大界面 / 缩小界面 / 重置缩放三项，点击先呼出主窗口再应用。
- **远程 IPC**：dsh UI 是远程源，Tauri 对远程源的 IPC 一律走 ACL（无 app manifest 时远程调用全部拒绝）。因此 build.rs 用 `AppManifest::commands` 声明全部 8 个命令（生成 `permissions/autogenerated/allow-*.toml`），`capabilities/dsh-remote.json` 只对 `http://127.0.0.1:*` 开放 `allow-zoom-ui` 一个命令。**副作用**：本地页面的 app 命令也转为 ACL 管控，default.json 已逐个 allow——**新增命令必须同步三处**：build.rs 的 commands 列表、capabilities/default.json（本地）、按需 dsh-remote.json（远程）。

## 10. 平台抽象

```rust
pub trait Platform: Send + Sync {
    fn node_exe_name(&self) -> &'static str;            // "node.exe" / "node"
    fn runtime_base_dir(&self) -> PathBuf;              // 应用数据根目录
    fn resource_runtime_dir(&self, resource_dir: &Path) -> PathBuf; // <res>/runtime/<triplet>
    fn runtime_triplet(&self) -> &'static str;          // "windows-x64" / "darwin-arm64" ...
    fn kill_process_tree(&self, pid: u32);              // taskkill /T /F
    fn configure_child_command(&self, _cmd: &mut Command) {} // CREATE_NO_WINDOW 等
    fn system_dark_mode(&self) -> bool;                 // 注册表 AppsUseLightTheme
}
```

`mod.rs` 对 macOS/Linux 是 `compile_error!` 占位——新增平台时编译器会强制你实现 trait 并接线 `current()`。配套还要做：`scripts/fetch-runtime.ps1` 支持对应 triplet 的 Node 下载、tauri.conf `bundle.targets` 加 dmg/appimage、CI matrix 打开对应行（`.github/workflows/build.yml` 注释里有清单）。主题着色在 `theme.rs` 里按 `cfg(windows)` 分支，其他平台走 `set_theme` 即可。

## 11. 打包与分发

### 运行时管线

```
scripts/fetch-runtime.ps1
  1. 下载 Node v24.19.0 win-x64 zip，只取 node.exe
  2. npm install --prefix dsh --omit=dev @deepseek-ai/dsh@0.1.0-rc.6
  3. 冒烟：node bin.js --help
  4. 调 scripts/prune-runtime.ps1 精简
产物：src-tauri/runtime/windows-x64/（gitignore，不入库）
```

`prune-runtime.ps1` 的规则（支持 `-WhatIf` 预演）：

- 通用：删 `test/tests/__tests__/docs/example/examples/coverage/.github` 等目录；删 `*.d.ts/*.map/*.md/LICENSE*/CHANGELOG*` 等文件；
- node-pty：只保留 `prebuilds/win32-x64`（删 darwin-*/win32-arm64 约 30MB），另删 src/deps/third_party 等；
- `@img/sharp-wasm32`（9MB）：sharp 已有 win32-x64 原生包时用不到，删。

效果：staging 344→227.9MB；安装包 **45.2MB**；安装后 **241.8MB**（大头是 node.exe ~90MB 与 dsh 依赖树，运行时与 node-pty 终端必需的 native 模块不能再删）。

### 安装包

- `pnpm tauri build` → NSIS `src-tauri/target/release/bundle/nsis/DSHDesktop_<ver>_x64-setup.exe`。
- WebView2：缺失时安装程序联网下载安装（downloadBootstrapper 模式），因此安装包本体不含 WebView2。
- 静默安装：`setup.exe /S`（加 `/D=<dir>` 指定目录）。升级前须先卸载旧版或结束运行实例。
- 国内构建机直连 GitHub 不稳时，NSIS 下载可用 ghproxy 预置 `%LOCALAPPDATA%\tauri\NSIS`（细节见 AGENTS.md）。

### CI 与发布

- `.github/workflows/build.yml`：tag `v*` 或手动触发 → windows-latest 上 fetch-runtime → `cargo test` → `tauri build` → 上传 artifact。
- `.github/workflows/release.yml`：tag `v*` 触发，构建后直接把 setup.exe + SHA256 发布到 GitHub Release。
- 版本号三处同步：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`。

## 12. 测试策略

| 层 | 内容 | 命令 |
| --- | --- | --- |
| Rust 单元测试（30） | runtime 部署/回退/路径归一化/复制进度回调、progress 阶段权重与百分比映射、theme BOM 解析、notify 过滤与摘要、LogRing 淘汰、port 分配与就绪探测、platform 基础、zoom 步进/clamp/持久化 | `cd src-tauri && cargo test` |
| 进程集成测试（2，tests/process.rs） | 用 `tests/fixtures/fake-dsh.cjs`（可脚本化崩溃的假 dsh）验证 就绪→HTTP 200→stop、崩溃→自动重启→二次 Ready | 同上 |
| 通知集成测试（1，tests/notify_ws.rs） | 本地 WS 服务器发事件帧，验证过滤与端口跟随 | 同上 |
| 控制台窗口回归（2，tests/console_window.rs） | 正组：CREATE_NO_WINDOW 的子进程**无可见** ConsoleWindowClass 窗口；对照组：CREATE_NEW_CONSOLE 的子进程**有**（证明检测有效，屏幕上会短暂弹真实控制台窗口，属正常） | 同上 |
| 端到端验收（scripts/acceptance.ps1） | 卸载旧版 → 静默安装 → 启动 → 等 dsh 就绪 → 单实例/无可见控制台/主题/截图 全项校验 | `powershell -File scripts/acceptance.ps1 -SetupExe <exe>` |

改进程/通知/主题逻辑后：`cargo test` + 重装走一遍 acceptance.ps1。

**调试手段优先级**：诊断面板（应用内） → `%LOCALAPPDATA%\DSHDesktop\events.log`（每个进程事件一行，1MB 截断；面板依赖应用内交互，卡启动时只有它能看） → `scripts/check-node.ps1` / `get-attr20.ps1` / `shot-window.ps1` 等外部脚本。

## 13. 已知限制与后续路线

- **Win10 深色标题栏聚焦纯黑**：系统行为，见 §8。路线：无边框 + 自绘标题栏（需处理 Win10 贴边分屏），暂缓。
- **通知覆盖面窄**：只有 approval/question 两类事件；dsh 上游接口稳定后再扩。
- **dsh 版本固定**：随应用版本钉死（fetch-runtime 的 `-DshVersion`），dsh 升级 = 发新版应用。将来可考虑应用内自选 dsh 通道。
- **UI 缩放为全局一份**：主窗口与诊断窗口共享同一缩放比例；快捷键固定 `Ctrl+Shift+=`/`-`，暂不可自定义。
- **仅 Windows x64**：平台抽象已就绪，见 §10 的扩展清单。

## 14. 附录：dsh 上游事实清单（0.1.0-rc.6）

| 事实 | 值 |
| --- | --- |
| npm 包 | `@deepseek-ai/dsh@0.1.0-rc.6` |
| Node 要求 | `^22.19 \|\| >=24`（随包内嵌 v24.19.0） |
| 入口 | `node_modules/@deepseek-ai/dsh/lib/bin.js` |
| Web 命令 | `bin.js web --port <N>`，仅绑 127.0.0.1 |
| 事件通道 | WebSocket `/api/events.mux`（GET → 426） |
| 事件帧 | `{"type":"server-request","method":"approval/requested"\|"question/requested","payload":{...}}` |
| 设置文件 | `$DSH_HOME/settings.yaml` → `ui-theme.preference: light\|dark\|system` |
| 信任栅栏 | 允许 loopback + 无 Origin 的 WS 连接 |
| 许可证 | MIT（Copyright 2026 DeepSeek） |
