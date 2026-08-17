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
| 事件通知走 WebSocket `/api/events.mux` + `/api/events.host` | dsh 官方事件下行通道 | 上游接口不稳定，需适配层隔离 |
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
│  lib.rs      组装：插件 → setup（代码创建主窗口）→ 事件桥（dsh 状态 → 前端事件/窗口导航）│
│  download.rs 主窗口下载处理：on_download 落系统下载目录 + 去重 + toast  │
│  presets.rs  启动期改写 shipped minimal 预设为 win32 pwsh 变体（签名门控）│
│  runtime.rs  运行时定位：原地运行 / 只读回退部署 / 旧副本清理               │
│  process.rs  DshProcess 监督循环：spawn、就绪探测、指数退避重启             │
│  notify/     WS 订阅 dsh 事件(mux+host 双下行) → 分类/台账 → 原生通知      │
│  theme.rs    轮询 dsh 主题设置 → DWM 标题栏着色                            │
│  progress.rs 首启进度模型：阶段权重、百分比映射、结构化事件负载             │
│  tray.rs     托盘菜单；diagnostics.rs 状态/日志；commands.rs 基础命令     │
│  zoom.rs     UI 缩放：可配置步进（默认 2%）、快捷键钩子注入、持久化        │
│  settings.rs 壳设置：settings.json 模型/校验/持久化 + get/set/试听命令      │
│  skills.rs   技能管理：skills/ ↔ skills-disabled/ 移动开关 + 三源导入      │
│  mcp.rs      MCP 管理：cordis.patch.yml 条目读写/启停 + 三源导入           │
│  remote/     远程访问：token 门岗反向代理 + cloudflared 隧道监督            │
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
  settings.json      壳设置（缩放步进/快捷键/关窗行为；缺失/损坏回退默认）
  runtime\           仅"只读安装目录"回退模式下存在（部署副本，带 .version 标记）
```

## 4. 启动时序

1. **插件初始化**：`single_instance` 必须最先注册——第二次启动时聚焦已有主窗口，不重复拉起。`window-state` 记忆窗口几何：缩放/移动实时入内存缓存，退出（`RunEvent::Exit`）时落盘 `%APPDATA%/<identifier>/.window-state.json`，下次启动建窗时恢复；flags 只取 `SIZE | POSITION | MAXIMIZED`——不含 `VISIBLE`，否则托盘隐藏态下退出会把"隐藏"记住，下次启动主窗口不出来。注意 restore 在插件 `window_created`（建窗后经 `run_on_main_thread` 排队）里执行，**晚于首批可见帧**（实测默认尺寸会可见 ~370ms）——所以主窗口以 `visible(false)` 创建、`on_page_load(Finished)` 时再 `show()`（此刻 restore 早已完成，首个可见帧即记忆几何）；回归见 `scripts/verify-no-size-flash.ps1`。注意主窗口自引入下载处理后改为 **setup 里代码创建**（`WebviewWindowBuilder`，tauri.conf `windows` 为空）：`on_download` 只能挂在 builder 上，conf 声明的窗口无法附加；`visible(false)+center()` 及 restore 时序不变（代码创建窗口同样在 `window_created` 排队 restore——托盘按需窗口一直如此）。
2. **setup**：代码创建主窗口（挂 `on_download`，见 §9）→ 建托盘 → 注册 `BootstrapInfo`（启动错误兜底）→ 取 `resource_dir` 定位内嵌运行时。
3. **首启判定**：`dsh-home` 在 ensure_runtime 之前不存在即首启（前端据此决定显示进度条还是纯文字）。
4. **ensure_runtime**（§5）：失败不退出，错误写入 `BootstrapInfo` 并 emit `dsh-progress`（stage=error），窗口停在启动画面显示错误（前端会主动 `get_bootstrap_error` 查询，因为错误事件可能早于前端 listen 注册而丢失）。
5. **seed_theme_preference → spawn_theme_follower**：首启时 settings.yaml 不存在则按系统深浅色预写 `ui-theme.preference`（dsh 缺省渲染浅色，不播种会出现"深标题栏 + 浅 UI"）；然后立即按系统主题给标题栏着色一次，进入 2s 轮询。
6. **spawn_supervised**：经 `tauri::async_runtime::block_on` 调用（setup 里没有 tokio 上下文，内部 `tokio::spawn` 依赖它）。状态机：`Starting → Ready{port}`。
7. **事件桥 bridge_event**（lib.rs）：所有 `dsh-progress` 事件都是结构化负载 `{stage, message, percent}`（progress.rs），百分比由后端按阶段权重计算：
   - `runtime`：原地运行 → 0→15%；回退部署 → 按复制字节实时报 0→70%（节流：百分比变化才发）
   - `starting`：15% 或 70%（取决于是否部署过）
   - `ready` → 100% → 写入 port 的 watch 通道（通知 WS 订阅器换端口）→ emit `dsh-ready` → **主窗口 navigate 到 `http://127.0.0.1:<port>/`**
   - `error` → 主窗口 navigate 回本地启动画面
   - 每个事件同时追加到 `events.log`。
8. dsh 就绪后，WS 订阅器连上 `/api/events.mux` 与 `/api/events.host`，进入稳态。

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
- **Job Object 防孤儿**：spawn 成功后立即 `Platform::register_child(pid)` 把子进程挂进全局 `KILL_ON_JOB_CLOSE` Job（`platform/windows.rs` 的 `job` 模块，句柄刻意永不关闭）。本进程以任何方式退出——包括被 NSIS 安装器/任务管理器强杀——内核都在最后句柄回收时连带终止全部成员及其子孙。0.1.8 之前没有这层保护：安装器只杀主程序，孤儿 node.exe/cloudflared.exe 锁住 runtime 目录导致重装中止（"Can't write: ...\cloudflared.exe"）。cloudflared 监督循环（remote/tunnel.rs）同样注册。
- **tokio 陷阱**：`Child::kill()` 返回 future，不 await 就不执行；泄漏的子进程若继承了 stdout 管道，外层等管道 EOF 会永远阻塞（集成测试曾因此假挂起）。所有子路径都必须 `kill_on_drop` + 显式 `child.wait().await` + 测试里 stdio 全 null。

## 7. 事件通知

```
dsh WS /api/events.mux ──▶ WsSource(mux) ──▶ handle_mux_frame ──┐
dsh WS /api/events.host ─▶ WsSource(host) ─▶ handle_host_frame ─▶ SessionBook
（两端点共用 WsSource：断线 5s 重连、端口经 watch 跟随重启换端口；   │（子代理集合
 host 重连先 clear_subagents——基线不可知，fail-open）              │ + 会话标题）
                                                                ▼
                                              NotifySink：窗口聚焦态 + 壳设置三类规则 → toast
```

- dsh 的事件帧：`{"type":"server-request","method":<payload.type>,"payload":{...}}`，mux/host 两端点同构（仅 WS；GET 返回 426）。
- mux 流三类放行：
  - `approval/requested` / `question/requested`（待批准/待回答，regex 粗筛）→ **Approval** / **Question** 通知（静音 toast，分别按 `notify.approval` / `notify.question` 规则门控）。
  - `session/event` 且 `event.type=="turn/end"` 且 `data.reason.kind=="completed"` → **TurnCompleted** 通知（可带提示音）；aborted/error/blocked/max-tokens 一律忽略。
  - `session/event` 且 `event.type=="session/title"` → 记入 SessionBook，完成通知正文带「会话标题」（无标题回退"dsh 回答完成"）。
- **两段式过滤**：先字符串 contains 粗筛、命中才 JSON 解析——流式期间每个 token chunk 都是一帧 `session/event`，不能逢帧解析。
- **子代理过滤**：mux 帧不含 origin；host 流的 `host/session-added`（`origin=="subagent"`）/ `host/session-removed` 维护子代理集合，命中的 turn/end 直接丢弃。子代理必然创建于 WS 连接之后（先创建再跑回合），时序天然安全；host 流不推基线，重连后集合清空（宁多弹一条，不漏弹）。
- dsh 的浏览器信任栅栏允许 loopback + 无 Origin 的请求，Rust 客户端天然满足。
- **适配层是有意为之**：`NotifySource` trait 隔离上游不稳定的接口，将来可加 `FileWatchSource`（解析 session jsonl）等替代实现。
- sink 弹通知前按类型查 `NotifyRule::allows(foreground)` 门控：**前台 = 本应用任一窗口（main/settings/diagnostics/skills/mcp/remote）处于聚焦态**（主窗口可见但失焦 = 用户已切走，算后台）；正在前台操作时不打扰，后台运行才弹（timing=always 的类型除外）。弹前写一行 `Notify: {kind} {body}` 到 events.log（通知链路的现场诊断抓手）。
- **通知提醒设置**（settings.json）：`notify.{approval,question,turn_done}` 三条规则，各为 `{enabled, timing}`——timing ∈ `background`（默认，仅无聚焦窗口时提醒）/ `always`（前台也提醒），三类默认均开。旧版 `notify_on_completion` 布尔在 load 时迁移进 `notify.turn_done.enabled`（读后即弃，保存时不再写出）。`completion_sound`（silent/default/im/mail/reminder/sms/chime/drop/mellow，默认 default）只作用于 TurnCompleted。音效透传 toast 音频预设（`ms-winsoundevent:Notification.*`，系统内置、不受用户声音方案影响；不传 sound 则 toast 静音——Approval/Question 类即如此）。试听走 `preview_completion_sound` 命令，弹一条带所选音效的 toast（音效是 toast 的属性，只能连通知一起听）。

## 8. 主题与语言跟随

目标：dsh 设置里的 `ui-theme.preference`（light/dark/system，存于 `$DSH_HOME/settings.yaml`）变化时，应用所有窗口的**标题栏**跟着变；本地页面（splash/诊断/设置/技能/MCP）与托盘菜单也同步；界面语言跟随 dsh 的 `locale.preference`（zh/en，缺省按系统 UI 语言）。

- **2s 轮询**配置文件（文件极小，轮询比 inotify 简单且跨平台无差异）；`system` 经注册表 `AppsUseLightTheme` 解析。语言同理读 `locale.preference`，缺省按 `GetUserDefaultUILanguage` 主语言 ID 是否中文（dsh 侧缺省"跟随浏览器"，WebView2 的浏览器语言同样来自系统）。
- **首启播种（seed_theme_preference）**：dsh 在 settings.yaml 缺失/无 preference 时**缺省渲染浅色 UI**，而壳标题栏缺省跟随系统——系统为深色时首启出现"深标题栏 + 浅内容"。ensure_runtime 之后、spawn_supervised 之前，若 settings.yaml 不存在则按系统深浅色预写 `ui-theme.preference`（dark/light，无 BOM），dsh 首启即与壳一致。已存在的文件绝不动。
- **本地页面**：页面加载时 `invoke('get_shell_ui_state')` 取快照、订阅 `shell-ui-state` 事件（2s 轮询解析值变化才广播）。`ui.svelte.ts` 把快照写入 `<html data-theme="dark|light">` + `color-scheme`；`app.css` 以 CSS 变量承载全部颜色（`:root` 暗色默认 + `html[data-theme='light']` 浅色覆盖），五个 Svelte 页面只引用变量。JS 未跑的 splash 首帧按 `@media (prefers-color-scheme: light)` 兜底渲染（与首启播种的"系统色即主题"一致）。
- **托盘菜单**：Windows 托盘右键菜单不响应 DWM 属性，用 uxtheme 未文档化 API `SetPreferredAppMode(ForceDark=2 / ForceLight=3)` + `FlushMenuThemes()`（tao 同源做法；uxtheme 常驻进程，LoadLibraryA 无需 FreeLibrary），在 `apply` 时随解析主题刷新。
- **BOM 陷阱（已修复，勿回退）**：PowerShell 5.1 的 `Set-Content -Encoding utf8` 会写入 UTF-8 BOM，而 yaml-rust 不接受 BOM——解析失败会**静默回退 system 主题**，表象是"标题栏永远白色"。`read_preference` 先剥 BOM 再解析。教训：**不要用 PowerShell 改写 settings.yaml**。
- **Windows 双管齐下**：
  1. `window.set_theme()` 同步 tao 内部主题状态——不同步的话，tao 可能在窗口事件后用缓存的旧状态覆盖可视效果。隐藏窗口上调用可能报错甚至 panic，必须 `catch_unwind` 兜住；
  2. 直接对 HWND 调 `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE=20)`——无缓存、幂等、对隐藏窗口同样生效，是标题栏颜色的权威来源。attr 20 失败（E_INVALIDARG）时回退旧值 19（Win10 20H1 之前）。
- **语言**：`theme.rs` 的轮询把解析后的 locale 写入 `i18n.rs` 全局原子（`i18n::pick(zh, en)` 取当前语言，托盘菜单/窗口标题/进度/通知/命令错误文案全部经它取）；locale 变化时重建托盘菜单（`tray::apply_locale`，Windows 托盘菜单不能改文案只能重建）并刷新本地窗口标题；前端 `i18n.ts` 以中文原文为 key 的 en 字典 + 响应式 `t()`（未命中 fallback 中文，避免裸 key）。`ShellUiState` 启动时立即写入全局语言、关注循环首轮 force 同步——否则 locale=en 时托盘菜单要等设置变化才重建。
- **已知限制**：Win10 上深色标题栏**聚焦时纯黑、失焦时深灰**是系统行为；`DWMWA_CAPTION_COLOR`(35)/`DWMWA_TEXT_COLOR`(36) 仅 Win11 可用。要做到恒为 dsh 的深灰（#1B1B1C），需要无边框窗口 + `initialization_script` 注入自绘标题栏——暂缓。

## 9. 前端与窗口管理

壳的本地页面只有四个，用 **hash 路由**（`App.svelte` 监听 `hashchange`）：

- `#/`（默认）**Splash.svelte**：启动画面。onMount 先 `invoke('get_bootstrap_error')` 主动查引导错误、`invoke('is_first_launch')` 查首启标记，再 listen 结构化的 `dsh-progress`。**首启时**显示分阶段进度条（百分比数字 + 阶段清单 ✓/●/○）与"首次启动需要部署运行时，可能要花几分钟"提示（仅此分支渲染，后续启动不出现）：runtime/starting 阶段百分比由后端给下限，`starting` 期间前端向 95% 渐近缓动（dsh 无细分进度信号，缓动只是呈现层，永不触顶），`ready` 到 100%。**非首启**维持纯文字 + 不确定滚动条。dsh 就绪后由 **Rust 侧**把主窗口 navigate 到 dsh UI——前端不自己跳。
- `#/diagnostics` **Diagnostics.svelte**：诊断面板（状态/端口/PID/版本、500 行实时日志回填 + `dsh-log` 事件流、重启按钮、开机自启开关）。
- `#/settings` **Settings.svelte**：其它设置（开机自启、关窗行为单选、三类通知提醒——任务确认/选项选择/回答完毕，各带启用勾选 + 仅后台时/总是时机下拉，回答完毕行关联完成提示音与试听、缩放步进 1%–25%、放大/缩小快捷键录制器）。保存时前端先校验（至少一个修饰键、in/out 不冲突），再 `invoke('set_shell_settings', { next })` 由 Rust 端复验并落盘。
- `#/skills` **Skills.svelte**：技能管理。数据源是**壳注入给 dsh 的 DSH_HOME**（`<runtime_base>/dsh-home`，不是 `~/.dsh`）：`skills/` 为启用、旁路 `skills-disabled/` 为停用（dsh 的技能发现只认根目录直属条目、无原生禁用概念；移出根目录即停用，watcher 观察到变化后热刷新 catalog，无需重启）。导入从三个外部 agent 的用户级源复制目录：Codex `~/.codex/skills`、Claude Code `~/.claude/skills`、OpenCode `~/.config/opencode/skills`；同名冲突逐个选覆盖/跳过（覆盖会同时清掉禁用目录里的旧副本）。**独立 dsh 的默认目录 `~/.dsh/skills` 不作为导入源**——壳就是 dsh，启动时自动扫描它并补入新技能（`skills::seed_from_default_dsh_home`；`.skills-seeded` marker 记录已见名字，壳里删掉的不会复活）。删除只删 home 内副本，不动源目录。Rust 侧 `skills.rs` 的 frontmatter 解析只取 description 单行键，行上操作均以目录名为准。
- `#/mcp` **Mcp.svelte**：MCP server 管理（列表/启停/删除/新增/编辑 + 导入）。dsh 没有独立的 mcp.json——MCP server 是 Cordis 插件补丁，壳读写 `<dsh-home>/profiles/web/cordis.patch.yml` 中 `name == '@deepseek-ai/dsh-mcp-client'` 的 insert 条目（只动这些条目，其余 Value 级保留；tmp+rename 原子写；读前剥 BOM）。dsh 的 HMR（`watchUserPatches` + chokidar）监听该文件，改后自动 disconnect+reconnect，**无需重启**。启停 = entry 上加/去 `disabled: true`（cordis-plugin-loader 原生语义，disabled 的 entry 不起 fiber）。编辑以旧 config 为底、只覆盖表单字段，`toolCallTimeoutMs`/`reconnect.*` 等高级键保留；transport 只有 `stdio`（command/args/env/cwd）与 `streamable-http`（url/headers）两种，sse 不支持。启动时种子同步 `~/.dsh` 两层 patch 里的 MCP 条目（`mcp::seed_from_default_dsh_home`，`.mcp-seeded` marker 防复活；源里 disabled 的不同步也不记 marker，日后在 ~/.dsh 启用时仍能进来）。手动导入三源：Claude Code `~/.claude.json` 的 `mcpServers`（stdio/http 映射，sse 标记"不支持"跳过）、Codex `~/.codex/config.toml` 的 `[mcp_servers.*]`（`enabled=false` 不列出）、OpenCode `~/.config/opencode/opencode.json` 的 `mcp` 段（local/remote 映射）；冲突逐个覆盖/跳过。patch 文件解析失败（如含无法处理的语法）时页面降级为只读并提示手工编辑。
- `#/remote` **Remote.svelte**：远程访问。状态取 `get_remote_status` 快照并订阅 `remote-status` 事件；Up 态显示二维码（`get_remote_qr` 返回 SVG）与完整链接，链接变化（隧道重连换域名）自动重取二维码；开关按钮按当前 phase 调 `start_remote` / `stop_remote`。

窗口行为：

- 主窗口 `main`：**关窗行为可配置**（settings.json 的 `close_behavior`）：默认 `background` = 隐藏到托盘（`CloseRequested` 时 `prevent_close` + `hide`），`quit` = 走托盘"退出"同一流程直接退出程序；托盘"打开主界面"或二次启动（单实例插件）时 `show` + `unminimize` + `set_focus`。
- 诊断窗口 `diagnostics`、设置窗口 `settings`、技能窗口 `skills`、MCP 窗口 `mcp`、远程访问窗口 `remote`：托盘菜单按需创建，**关窗 = 销毁**，下次再建。
- 托盘"退出"：先 `stop()` 远程访问（杀 cloudflared 进程树 + 关停鉴权代理，链接即刻失效），再 `stop()` dsh，等 1.5s 让监督循环杀完进程树，最后 `exit(0)`。
- 导航到远程 URL 后窗口标题被 dsh 的 `document.title` 覆盖——**外部脚本不要按标题找窗口**（按 PID + 类名，见 `scripts/shot-window.ps1`）。
- **首次启动居中**：主窗口（setup 里 builder `.center()`，tauri.conf `windows` 已空）与托盘按需创建的五个窗口都以屏幕居中为默认位置；window-state 插件的 restore 在 window_created 时排队、早于首个可见帧执行，有记忆几何时覆盖居中默认值——首次启动居中、之后按上次位置，居中默认不会闪一帧再跳变（verify-no-size-flash.ps1 探针断言首个可见帧即记忆几何）。
- **下载处理（download.rs）**：WebView2 的下载不接管则静默消失——wry 默认 handler 放行但 `SetHandled(true)` 抑制了下载 UI，用户看不到文件去向（dsh "Session log" 导出即受此影响）。主窗口 builder 挂 `on_download`：Requested 时把目标改到系统下载目录（`dirs::download_dir`，已存在则追加 " (n)" 序号防覆盖），Finished 时按成败弹 toast 告知落盘路径；全程记 events.log（`Download: requested/finished`）。

IPC 命令：commands.rs 8 个——`get_shell_ui_state` / `get_status` / `restart_dsh` / `get_recent_logs` / `get_autostart` / `set_autostart` / `get_bootstrap_error` / `is_first_launch`；另有 zoom.rs 的 `zoom_ui`、settings.rs 的 `get_shell_settings` / `set_shell_settings` / `preview_completion_sound`、skills.rs 的 `list_skills` / `list_import_sources` / `import_skills` / `set_skill_enabled` / `delete_skill`、mcp.rs 的 `list_mcp_servers` / `upsert_mcp_server` / `set_mcp_enabled` / `delete_mcp_server` / `list_mcp_import_sources` / `import_mcp_servers`、remote/mod.rs 的 `start_remote` / `stop_remote` / `get_remote_status` / `copy_remote_link` / `get_remote_qr`（共 28 个，见下）。

壳设置（settings.rs）：

- **模型**：`settings.json` 存 `zoom_step`（0.01–0.25，越界 clamp）、`zoom_in`/`zoom_out` 快捷键（`{ctrl, shift, alt, code, key}`）、`close_behavior`（`background`/`quit`）、`notify`（`{approval, question, turn_done}` 三条 `{enabled, timing}` 规则，默认全开、仅后台时提醒；旧版 `notify_on_completion` 布尔读取时迁移进 `notify.turn_done.enabled`，保存时不再写出）、`completion_sound`（`silent`/`default`/`im`/`mail`/`reminder`/`sms`/`chime`/`drop`/`mellow`，默认 `default`）。缺失/损坏 → 全默认；部分字段缺失 → 逐字段回退默认（serde default）；校验失败（无修饰键/in-out 冲突）→ 全默认，不带坏状态跑。
- **SettingsState**：托管内存值 + 持久化目录；`set` 先 clamp/校验再落盘再替换内存，校验失败则内存磁盘都保持旧值。
- **保存即生效**：`set_shell_settings` 成功后对主窗口重注入缩放钩子（快捷键定义内嵌在脚本里必须重注入）；步进不写死在脚本里，`zoom_ui` 调用时从设置读，改步进本来就无需重注入。

UI 缩放（zoom.rs）：

- **快捷键**：默认 `Ctrl+Shift+=` 放大、`Ctrl+Shift+-` 缩小（可在设置窗口自定义），步进默认 ±2 个百分点（可配 1%–25%，clamp 到 25%–500%）。钩子脚本由 `hook_js(&ShellSettings)` 生成——快捷键定义内嵌为 JSON，匹配逻辑与 `Shortcut::matches` 对齐：`e.code` 物理键位为主，`e.key` 兜底（合成按键与 RDP 注入的 keydown `e.code` 为空，纯 code 匹配会整组失效），meta 永不命中。`on_page_load` 在每次整页加载完成后 eval 注入（**只注入 main 窗口**——设置窗口录制快捷键时不能被钩子抢先拦截；本地 splash 与远程 dsh UI 通用），capture 阶段拦截并 invoke `zoom_ui`（负载 `direction: "in"/"out"`），经 WebView2 原生 `SetZoomFactor` 生效——与浏览器 Ctrl++ 同一机制。监听器可热替换（`__dshZoomHookHandler` 存旧 handler，重注入先 `removeEventListener` 再挂新的，不叠加）。
- **持久化**：每次变更即写 `%LOCALAPPDATA%\DSHDesktop\ui-zoom.txt`；缺失/损坏回退 100%；每次页面加载时 `on_page_load` 统一重应用当前缩放（兼作 WebView2 重建后的兜底）。
- **远程 IPC**：dsh UI 是远程源，Tauri 对远程源的 IPC 一律走 ACL（无 app manifest 时远程调用全部拒绝）。因此 build.rs 用 `AppManifest::commands` 声明全部 28 个命令（生成 `permissions/autogenerated/allow-*.toml`），`capabilities/dsh-remote.json` 只对 `http://127.0.0.1:*` 开放 `allow-zoom-ui` 一个命令。**副作用**：本地页面的 app 命令也转为 ACL 管控，default.json 已逐个 allow——**新增命令必须同步三处**：build.rs 的 commands 列表、capabilities/default.json（本地）、按需 dsh-remote.json（远程）。

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
- 静默安装：`setup.exe /S`（加 `/D=<dir>` 指定目录）。运行中的旧实例由安装器自动结束，无需手动卸载或杀进程。
- NSIS 钩子（`src-tauri/windows/nsis-hooks.nsh`，经 `bundle.windows.nsis.installerHooks` 接入，**路径相对 `src-tauri`**）：`NSIS_HOOK_PREINSTALL`/`NSIS_HOOK_PREUNINSTALL` 先 `taskkill /F /T /IM DSHDesktop.exe` 杀整棵树，再用 PowerShell 按可执行路径清扫 `$INSTDIR` 下的所有残留进程（≤0.1.8 遗留的孤儿 node.exe/cloudflared.exe），最后 Sleep 1.5s 等内核回收句柄。Tauri 模板自带的 `CheckIfAppIsRunning` 只杀主程序，杀不动子进程，单靠它必然复现 "Can't write" 失败。
- 国内构建机直连 GitHub 不稳时，NSIS 下载可用 ghproxy 预置 `%LOCALAPPDATA%\tauri\NSIS`（细节见 AGENTS.md）。

### CI 与发布

- `.github/workflows/build.yml`：tag `v*` 或手动触发 → windows-latest 上 fetch-runtime → `cargo test` → `tauri build` → 上传 artifact。
- `.github/workflows/release.yml`：tag `v*` 触发，构建后直接把 setup.exe + SHA256 发布到 GitHub Release。
- 版本号三处同步：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`。

## 12. 测试策略

| 层 | 内容 | 命令 |
| --- | --- | --- |
| Rust 单元测试（99） | runtime 部署/回退/路径归一化/复制进度回调、progress 阶段权重与百分比映射、theme BOM 解析与首启播种、notify 帧分类/子代理台账/摘要、LogRing 淘汰、port 分配与就绪探测、platform 基础、zoom clamp/持久化/钩子脚本内嵌设置、settings 模型/校验/持久化/提示音枚举/通知规则门控与旧键迁移、skills frontmatter 解析/列表/启停/删除/导入冲突、mcp patch 解析/启停/删除/upsert 校验与高级键保留/种子 marker/三源解析与导入冲突、remote 隧道 URL 解析与 token 脱敏 | `cd src-tauri && cargo test` |
| 进程集成测试（2，tests/process.rs） | 用 `tests/fixtures/fake-dsh.cjs`（可脚本化崩溃的假 dsh）验证 就绪→HTTP 200→stop、崩溃→自动重启→二次 Ready | 同上 |
| 通知集成测试（2，tests/notify_ws.rs） | fixture 双 WS 端点发事件帧，验证 approval 过滤、turn/end 完成通知（含标题）、子代理过滤 | 同上 |
| 远程访问集成测试（12，tests/remote_{proxy,tunnel,manager}.rs） | 门岗 403/302/cookie/转发/浏览器标记头剥离（防 dsh 信任栅栏 403）/WS 桥接/503/停服释放端口、fake-cloudflared URL 解析与崩溃重启、manager 全链路（缺文件 error、up→stop、start 幂等） | 同上 |
| 控制台窗口回归（2，tests/console_window.rs） | 正组：CREATE_NO_WINDOW 的子进程**无可见** ConsoleWindowClass 窗口；对照组：CREATE_NEW_CONSOLE 的子进程**有**（证明检测有效，屏幕上会短暂弹真实控制台窗口，属正常） | 同上 |
| 端到端验收（scripts/acceptance.ps1） | 卸载旧版 → 静默安装 → 启动 → 等 dsh 就绪 → 单实例/无可见控制台/主题/截图 全项校验 | `powershell -File scripts/acceptance.ps1 -SetupExe <exe>` |

改进程/通知/主题逻辑后：`cargo test` + 重装走一遍 acceptance.ps1。

**调试手段优先级**：诊断面板（应用内） → `%LOCALAPPDATA%\DSHDesktop\events.log`（每个进程事件一行，1MB 截断；面板依赖应用内交互，卡启动时只有它能看） → `scripts/check-node.ps1` / `get-attr20.ps1` / `shot-window.ps1` 等外部脚本。

## 13. 已知限制与后续路线

- **Win10 深色标题栏聚焦纯黑**：系统行为，见 §8。路线：无边框 + 自绘标题栏（需处理 Win10 贴边分屏），暂缓。
- **通知覆盖**：approval/question + 回合正常完成（turn/end/completed，可带提示音）；任务出错（kind==error）暂不提醒。子代理过滤依赖 events.host 增量帧，host 重连窗口期内可能多弹一条（fail-open）。其余事件类型待 dsh 上游接口稳定后再扩。
- **dsh 版本固定**：随应用版本钉死（fetch-runtime 的 `-DshVersion`），dsh 升级 = 发新版应用（跟版流程见 §14）。将来可考虑应用内自选 dsh 通道。
- **UI 缩放只作用于主窗口**：诊断/设置窗口不注入钩子、不应用缩放值；快捷键与步进均可在"其它设置"中自定义。
- **fs-local 列目录遇 ACL 拒绝项即整列失败**：上游行为——列举目录时逐个子项解析，任一子项权限被拒（如 `C:\\` 根目录的 `DumpStack.log`、`C:\\Users` 下他人配置目录）整个列表报 `cannot list ...: permission denied`。Windows 上列系统盘根目录必现。壳侧不修它，缓解是让模型知道并待在自己的 workspace（极简模式的 persona 已补工作目录事实）。
- **仅 Windows x64**：平台抽象已就绪，见 §10 的扩展清单。

## 14. 更新策略（跟随 dsh 上游）

上游源仓库：[deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（npm 包 `@deepseek-ai/dsh`）。壳不 fork、不打补丁、不改 dsh 源码（§1 非目标），只做跟版发版。**唯一例外**是 presets.rs 的极简模式 Windows 修复：上游 rc 的 minimal 预设无条件挂载 PTY 持久 bash，而终端检查器未实现 win32，且 composeProfile 会把 agent-presets 行的 roots 无条件重写为 shipped root、profile patch 层无法注入影子根——只能启动期原地改写 shipped 预设文件（配置组合而非代码）。该补丁签名门控（上游加了 win32 分支即自动停手）、幂等、随 dsh 自更新还原后重打；上游修复后应整体移除。dsh 的内部优化（启动速度、UI 迭代等）对壳透明，重打包即受益。

**版本钉死**：dsh 随应用版本钉死在安装包里（`fetch-runtime.ps1` 的 `-DshVersion`），用户机器上的 dsh 不会自动更新——**dsh 升级 = 我们发一版新应用**。

**跟版流程（常规升级为纯流程，零代码改动）**：

1. 关注上游 release 与 npm 版本流，对照 §15 事实清单评估是否触及接口契约。
2. 改 `scripts/fetch-runtime.ps1` 的 `-DshVersion` 默认值，重新抓运行时（含冒烟与 prune 精简）；若上游新增依赖（尤其 native 模块、多平台 prebuilds），按需调整 `prune-runtime.ps1` 规则。
3. `cd src-tauri && cargo test` 全绿 → `pnpm tauri build` → `scripts/acceptance.ps1` 全项验收。
4. 三处版本号同步（§11）后打 tag `v*`，CI 自动构建并发布 GitHub Release。

**接口契约核对表（上游变了才动代码）**：

| 上游事实（当前值见 §15） | 变了要动哪里 |
| --- | --- |
| 入口 `lib/bin.js` 路径 | `runtime.rs` 的 `paths_for` / `validate_source` |
| `web --port <N>` 命令形式 | `process.rs` 的 spawn 参数 |
| 事件通道 `/api/events.mux` + `/api/events.host` 及帧格式 | `notify/` 适配层（`NotifySource` trait 即为此隔离；帧分类在 `handle_mux_frame`/`handle_host_frame`） |
| `settings.yaml` 的 `ui-theme.preference` | `theme.rs`（解析 + 首启播种） |
| Node 版本要求 | `fetch-runtime.ps1` 的 `-NodeVersion` |
| WS 信任栅栏（loopback / 无 Origin） | `notify/ws.rs` 握手 |

契约变化大多会在 `cargo test`（WS 通知集成、主题解析等单测）或 acceptance 全链路中暴露；现场问题先看 `events.log`。

**回滚**：新版 dsh 出严重问题、壳又要先发补丁时，`-DshVersion` 回退到上一可用版本重打包即可——用户数据全在 `dsh-home`，与 dsh 版本解耦。

## 15. 附录：dsh 上游事实清单（0.1.0-rc.6）

| 事实 | 值 |
| --- | --- |
| npm 包 | `@deepseek-ai/dsh@0.1.0-rc.6` |
| Node 要求 | `^22.19 \|\| >=24`（随包内嵌 v24.19.0） |
| 入口 | `node_modules/@deepseek-ai/dsh/lib/bin.js` |
| Web 命令 | `bin.js web --port <N>`，仅绑 127.0.0.1 |
| 事件通道 | WebSocket `/api/events.mux` + `/api/events.host`（GET → 426，仅 WS） |
| 事件帧 | `{"type":"server-request","method":<payload.type>,"payload":{...}}`；完成判定用 `session/event` 里的 `turn/end`（`data.reason.kind`），子代理标记用 `host/session-added` 的 `origin` |
| 设置文件 | `$DSH_HOME/settings.yaml` → `ui-theme.preference: light\|dark\|system` |
| 信任栅栏 | 允许 loopback + 无 Origin 的 WS 连接 |
| Agent 预设 | `config/agent-presets/{minimal,standard,code,cordis}`；standard/code/cordis 的 shell 工具有 win32 分支（tool-pwsh），minimal 无分支（PTY 持久 bash，win32 必败）→ 壳 presets.rs 启动期改写 |
| 预设根 | composeProfile 把 agent-presets 行 roots 强制重写为 shipped root（`config/agent-presets/`）；`$DSH_HOME/.agent-presets` 用户根追加在后，同名 id shipped 优先 |
| WebView2 下载 | 宿主不处理 DownloadStarting 即静默取消；wry 默认放行且抑制下载 UI → 壳 download.rs 显式接管 |
| 许可证 | MIT（Copyright 2026 DeepSeek） |

## 16. 远程访问（Quick Tunnel + 内嵌鉴权代理）

托盘"远程访问"一键开启后，手机/异地浏览器凭带 token 的链接获得**完整 dsh Web UI**。零服务器、零账号、零配置：中继用 Cloudflare 免费 Quick Tunnel（`cloudflared.exe` 随 runtime 内嵌，匿名临时隧道，纯出站连接，无需公网 IP/端口映射/防火墙开口）。

**链路**：

```
手机浏览器 ─HTTPS→ Cloudflare 边缘 ─→ cloudflared(桌面，纯出站)
  → 127.0.0.1:<随机端口> remote::proxy(token 门岗)
  → 127.0.0.1:<dsh端口>  dsh web（HTTP + /api/events.* WS）
```

**模块**（`src-tauri/src/remote/`）：

- `mod.rs` — `RemoteManager`：生命周期（start/stop/status）、token 生成（每次 start 重新生成 256-bit hex）、6 个 invoke 命令（start_remote/stop_remote/get_remote_status/copy_remote_link/get_remote_qr/reset_remote_link）。隧道事件回调里拼链接 `link = {url}/?token={token}`；状态变更广播 `remote-status` 事件 + 更新托盘子菜单 enabled + events.log。`reset_link` 原地轮换 token 并掐断现有会话（链接泄露后的吊销手段），仅 Starting/Up 可用，隧道与域名不变。
- `proxy.rs` — axum 反向代理，只绑 127.0.0.1。鉴权：有效 cookie `__dsh_remote` 直接转发；`?token=` 匹配（常数时间比较）→ 302 剥离 token + 种 HttpOnly cookie；token 不匹配 → 固定 500ms 延迟后 403；无凭据 → 403 门页。HTTP 经 reqwest 流式转发（3xx 透传不跟随）；WS upgrade 在代理终结握手后与 dsh 另建连接逐帧双向桥接。dsh 端口走 `watch::Receiver` 动态读取，dsh 重启代理不断线。**token 存共享单元（RwLock），门岗逐请求读最新值**——重置后旧链接/旧 cookie 即刻失效；**WS 桥接同时挂 drain Notify**（`enable()` 提前挂号防 connect 窗口期漏掐），重置/停服 `notify_waiters` 掐断所有已建立连接，否则泄露场景下攻击者已开的页面仍能持续收事件流。**转发必须剥掉浏览器标记头**（`origin`/`referer`/`sec-fetch-*`）：dsh 的 /api 信任栅栏（dsh-client-connection `isTrustedApiRequest`）要求 Origin.host == Host 头且拒绝 `sec-fetch-site: cross-site`，隧道场景 Origin 是 trycloudflare 域名，不剥则页面所有 RPC 调用全 403；剥掉后请求在 dsh 眼里是无 Origin 的 loopback 客户端（WS 桥接侧 tungstenite 握手本就不带 Origin，天然满足）。**转发客户端必须 `.no_proxy()`**——用户系统代理（Clash 等）否则会把 127.0.0.1 转发劫持走。**插件 bundle 改写**：`/plugins/*/client.js` 响应被缓冲（≤4MB、仅 identity 编码）并把 `connection.isLoopback ? "host" : "memory"` 全量替换为 `"host"`——dsh 内测声明（WelcomeNoticeStore）对非回环源选 memory 持久化，隧道域名下确认记录不落 settings.yaml 导致每次连接都弹；改写后远程端与桌面端共用 host 持久化（桌面本就已确认，远程直接不弹）。改写路径转发时剥 `accept-encoding`（求 identity）与条件请求头（防 304），响应剥 `content-length`/`etag`；needle 失配（dsh 改版换写法）静默原样透传，声明照弹但不破坏页面。
- `tunnel.rs` — `TunnelProcess` 监督（对齐 DshProcess 模式）：`cloudflared tunnel --url <代理地址> --no-autoupdate`，从 stdout 正则解析 `https://<rand>.trycloudflare.com`（60s 未出现视为失败），指数退避重启（隧道重连后**域名变、token 不变**），停止走 `kill_process_tree` + `kill_on_drop`。

**安全模型**：链接即凭据（托盘/二维码页有"勿分享"提示）；token 每次开启重新生成，停止/退出应用即整体失效（quit 顺序：先 stop 远程访问再 stop dsh）；泄露时"重置链接"一键吊销（token 轮换 + 掐断现有会话，域名不变）。代理与 dsh 均不监听非 loopback。**token 不落日志**：events.log 只记 phase/url/error/proxy_port（不含链接），cloudflared 输出里的 `?token=` 查询串经 `redact_token` 脱敏；开启成功的 toast 正文不带链接（系统通知中心会留痕），只提示去托盘复制。已知取舍：链接不固定，手机端不能收藏复用（每次开启重新扫码），换"泄露窗口期最短"。

**分发**：`fetch-runtime.ps1 -CloudflaredVersion`（默认见脚本）从 GitHub release 下载 `cloudflared-windows-amd64.exe`（ghproxy 兜底），落 `runtime/<triplet>/cloudflared.exe`，经既有 `resources: ["runtime"]` 打包与 `.version` 部署比对；缺失时 start 报 error 态（dev 的 fixture 运行时允许没有）。

**UI**：托盘子菜单（开启/关闭互斥 enabled、复制链接、显示二维码、重置远程链接——复制/二维码/重置仅 Up 可用）+ `#/remote` 本地窗口（二维码 SVG 由 `qrcode` crate 生成、复制、重置（confirm 确认）、开关按钮）+ 诊断面板"远程访问"状态行。locale 切换重建托盘菜单后 `TrayRemoteItems` 句柄替换并按当前 phase 重设 enabled。

**测试**：`tests/remote_proxy.rs`（门岗 403/302/种 cookie/转发/WS 桥接/503/shutdown 释放端口/插件 bundle 三元式改写）、`tests/remote_tunnel.rs`（fake-cloudflared.cjs：URL 解析、崩溃重启、stop）、`tests/remote_manager.rs`（缺 cloudflared 报 error、fixture dsh + 假隧道全链路 up→stop）。真隧道链路不进自动化（需外网），手动验收：托盘开启 → 手机扫码完整操作 dsh。
