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
│  skills.rs   技能管理：skills/ ↔ skills-disabled/ 移动开关 + 三源/ZIP 导入   │
│  mcp.rs      MCP 管理：cordis.patch.yml 条目读写/启停 + 三源导入           │
│  picker.rs   目录选择器钉 browse：幂等写 cordis.patch.yml（禁 auto 行       │
│              + insert browse 对），手机远程端才能选文件夹                  │
│  pickerpatch.rs browse 选择器包内文件原地补丁：host 加 "dsh:drives" 盘符    │
│              哨兵层级，client 隐藏条目默认显示+哨兵面包屑本地化/禁用打开     │
│  welcome.rs  内测声明豁免播种：预写 ui-onboarding.welcomeNoticeVersion      │
│  remote/     远程访问：token 门岗反向代理 + 固定端口局域网暴露 + SSH 隧道  │
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

**背景**：安装包把运行时原样放在 `<install>\runtime\windows-x64\`（tauri.conf `resources` 映射：`"runtime" -> "runtime"`、`"resources/sounds" -> "sounds"`；列表形式会把 `resources/sounds` 原样放到 `<install>/resources/sounds/`，而壳在 exe 旁找 `sounds/*.wav`（`resolve_custom_sound`），探测不到自定义提示音就静默降级系统默认——0.1.16 实踩，`settings.rs` 有锚定测试）。早期设计是首次启动时把整个运行时复制到 `%LOCALAPPDATA%`（防只读安装目录），代价是安装后体积翻倍（约 +230MB）。

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
- **Job Object 防孤儿**：spawn 成功后立即 `Platform::register_child(pid)` 把子进程挂进全局 `KILL_ON_JOB_CLOSE` Job（`platform/windows.rs` 的 `job` 模块，句柄刻意永不关闭）。本进程以任何方式退出——包括被 NSIS 安装器/任务管理器强杀——内核都在最后句柄回收时连带终止全部成员及其子孙。0.1.8 之前没有这层保护：安装器只杀主程序，孤儿 node.exe 锁住 runtime 目录导致重装中止（"Can't write: ...\node.exe"）。
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
- **非客户区必须强制重绘（已修复，勿回退）**：`DwmSetWindowAttribute` 只改属性、不重绘标题栏——窗口会保持旧色直到下一次激活（用户"点一下才变色"）；tao `set_theme` 内部伪造 `WM_NCACTIVATE` 触发重绘，但该法在部分时序/焦点状态下不生效（winit/Electron 均因此弃用）。主题实际变化时（`LAST_APPLIED` 原子门控，轮询同值不重复强制，防每 2s 闪一次）对每个窗口 `SetWindowPos(SWP_FRAMECHANGED|SWP_NOACTIVATE|NOMOVE|NOSIZE|NOZORDER)` + `RedrawWindow(RDW_FRAME|RDW_INVALIDATE|RDW_UPDATENOW)` 强制非客户区重绘（Chromium/Windows Terminal 同款），并在 events.log 留痕。另外新建窗口的 DWM 属性来自系统主题（tao 建窗行为），"系统浅色 + dsh 深色"组合下会带错色出生；`on_page_load` 在 show 前经 `apply_before_show` 把属性落对（已可见的主窗口整页导航则同时强制重绘）。回归：`scripts/verify-titlebar-theme.ps1`（不点击窗口断言标题栏像素即时换色）。
- **语言**：`theme.rs` 的轮询把解析后的 locale 写入 `i18n.rs` 全局原子（`i18n::pick(zh, en)` 取当前语言，托盘菜单/窗口标题/进度/通知/命令错误文案全部经它取）；locale 变化时重建托盘菜单（`tray::apply_locale`，Windows 托盘菜单不能改文案只能重建）并刷新本地窗口标题；前端 `i18n.ts` 以中文原文为 key 的 en 字典 + 响应式 `t()`（未命中 fallback 中文，避免裸 key）。`ShellUiState` 启动时立即写入全局语言、关注循环首轮 force 同步——否则 locale=en 时托盘菜单要等设置变化才重建。
- **已知限制**：Win10 上深色标题栏**聚焦时纯黑、失焦时深灰**是系统行为；`DWMWA_CAPTION_COLOR`(35)/`DWMWA_TEXT_COLOR`(36) 仅 Win11 可用。要做到恒为 dsh 的深灰（#1B1B1C），需要无边框窗口 + `initialization_script` 注入自绘标题栏——暂缓。

## 9. 前端与窗口管理

壳的本地页面只有四个，用 **hash 路由**（`App.svelte` 监听 `hashchange`）：

- `#/`（默认）**Splash.svelte**：启动画面。onMount 先 `invoke('get_bootstrap_error')` 主动查引导错误、`invoke('is_first_launch')` 查首启标记，再 listen 结构化的 `dsh-progress`。**首启时**显示分阶段进度条（百分比数字 + 阶段清单 ✓/●/○）与"首次启动需要部署运行时，可能要花几分钟"提示（仅此分支渲染，后续启动不出现）：runtime/starting 阶段百分比由后端给下限，`starting` 期间前端向 95% 渐近缓动（dsh 无细分进度信号，缓动只是呈现层，永不触顶），`ready` 到 100%。**非首启**维持纯文字 + 不确定滚动条。dsh 就绪后由 **Rust 侧**把主窗口 navigate 到 dsh UI——前端不自己跳。
- `#/diagnostics` **Diagnostics.svelte**：诊断面板（状态/端口/PID/版本、500 行实时日志回填 + `dsh-log` 事件流、重启按钮、开机自启开关）。
- `#/settings` **Settings.svelte**：其它设置（开机自启、关窗行为单选、三类通知提醒——任务确认/选项选择/回答完毕，各带启用勾选 + 仅后台时/总是时机下拉，回答完毕行关联完成提示音与试听、缩放步进 1%–25%、放大/缩小快捷键录制器）。保存时前端先校验（至少一个修饰键、in/out 不冲突），再 `invoke('set_shell_settings', { next })` 由 Rust 端复验并落盘。
- `#/skills` **Skills.svelte**：技能管理。数据源是**壳注入给 dsh 的 DSH_HOME**（`<runtime_base>/dsh-home`，不是 `~/.dsh`）：`skills/` 为启用、旁路 `skills-disabled/` 为停用（dsh 的技能发现只认根目录直属条目、无原生禁用概念；移出根目录即停用，watcher 观察到变化后热刷新 catalog，无需重启）。导入从三个外部 agent 的用户级源复制目录：Codex `~/.codex/skills`、Claude Code `~/.claude/skills`、OpenCode `~/.config/opencode/skills`；同名冲突逐个选覆盖/跳过（覆盖会同时清掉禁用目录里的旧副本）。**独立 dsh 的默认目录 `~/.dsh/skills` 不作为导入源**——壳就是 dsh，启动时自动扫描它并补入新技能（`skills::seed_from_default_dsh_home`；`.skills-seeded` marker 记录已见名字，壳里删掉的不会复活）。删除只删 home 内副本，不动源目录。还可本地导入 ZIP 压缩包（`inspect_zip_skills`/`import_zip_skills`）：自动识别两种布局——包根直接含 SKILL.md（名字取 frontmatter name，缺失回退 zip 文件名）或顶层若干技能文件夹各含 SKILL.md；解包剥掉顶层前缀，条目路径经 enclosed_name 过滤防 zip-slip，另有 1 万条目/256MB 上限防 zip 炸弹；冲突语义与目录导入一致（跳过/覆盖，覆盖清两侧）。Rust 侧 `skills.rs` 的 frontmatter 解析只取单行键，行上操作均以目录名为准。
- `#/plugins` **Plugins.svelte**：插件管理（npm/cordis 插件的图形化装/卸/更新，详见 §18）。
- `#/mcp` **Mcp.svelte**：MCP server 管理（列表/启停/删除/新增/编辑 + 导入）。dsh 没有独立的 mcp.json——MCP server 是 Cordis 插件补丁，壳读写 `<dsh-home>/profiles/web/cordis.patch.yml` 中 `name == '@deepseek-ai/dsh-mcp-client'` 的 insert 条目（只动这些条目，其余 Value 级保留；tmp+rename 原子写；读前剥 BOM）。dsh 的 HMR（`watchUserPatches` + chokidar）监听该文件，改后自动 disconnect+reconnect，**无需重启**。启停 = entry 上加/去 `disabled: true`（cordis-plugin-loader 原生语义，disabled 的 entry 不起 fiber）。编辑以旧 config 为底、只覆盖表单字段，`toolCallTimeoutMs`/`reconnect.*` 等高级键保留；transport 只有 `stdio`（command/args/env/cwd）与 `streamable-http`（url/headers）两种，sse 不支持。启动时种子同步 `~/.dsh` 两层 patch 里的 MCP 条目（`mcp::seed_from_default_dsh_home`，`.mcp-seeded` marker 防复活；源里 disabled 的不同步也不记 marker，日后在 ~/.dsh 启用时仍能进来）。手动导入三源：Claude Code `~/.claude.json` 的 `mcpServers`（stdio/http 映射，sse 标记"不支持"跳过）、Codex `~/.codex/config.toml` 的 `[mcp_servers.*]`（`enabled=false` 不列出）、OpenCode `~/.config/opencode/opencode.json` 的 `mcp` 段（local/remote 映射）；冲突逐个覆盖/跳过。patch 文件解析失败（如含无法处理的语法）时页面降级为只读并提示手工编辑。
- `#/remote` **Remote.svelte**：远程访问。状态取 `get_remote_status` 快照并订阅 `remote-status` 事件；Up 态显示二维码（`get_remote_qr` 返回 SVG）与完整链接，链接变化（每次开启 token 换新）自动重取二维码；开关按钮按当前 phase 调 `start_remote` / `stop_remote`。

窗口行为：

- 主窗口 `main`：**关窗行为可配置**（settings.json 的 `close_behavior`）：默认 `background` = 隐藏到托盘（`CloseRequested` 时 `prevent_close` + `hide`），`quit` = 走托盘"退出"同一流程直接退出程序；托盘"打开主界面"、**左键单击托盘图标**（`on_tray_icon_event` 的 Left/Up；`show_menu_on_left_click(false)`，菜单改走右键）或二次启动（单实例插件）时 `show` + `unminimize` + `set_focus`——窗口只是隐藏未销毁，位置保持隐藏前状态。
- `mod.rs` — `RemoteManager`：生命周期（start/stop/status）、token 生成（每次 start 重新
  生成 256-bit hex）、6 个 invoke 命令（start_remote/stop_remote/get_remote_status/
  copy_remote_link/get_remote_qr/reset_remote_link）。start 时读运行配置（`RemoteConfig`
  watch 通道：固定端口 + SSH 隧道配置，settings.rs 保存设置即更新，无需重启应用），把
  鉴权代理绑到 `0.0.0.0:<端口>`；SSH 模式再拉起反向隧道并**等隧道就绪才转 Up**（避免
  链接 URL 对外但隧道未建立），链接 = `http://<服务器地址>:<暴露端口>/?token={token}`
  （协议跟随服务器地址前缀：带 https:// 则链接走 https；端口可用 link_port 覆盖——
  自建服务器用反向代理对外公布、对外端口 ≠ SSH 转发端口时填对外端口，隧道 -R 绑定的
  仍是 expose_port，两者解耦），否则 = `http://<局域网IP>:<端口>/?token=…`
  （局域网 IPv4 用 UDP connect 选默认路由接口，不发包）。端口被占用/SSH 配置不完整 →
  error 态并提示。状态变更广播 `remote-status` 事件 + 更新托盘子菜单 enabled + events.log。
  `reset_link` 原地轮换 token 并掐断现有会话（链接泄露后的吊销手段），地址与端口不变。
- `ssh_tunnel.rs` — SSH 反向隧道监督：`ssh -N -T -o BatchMode=yes -o ExitOnForwardFailure=yes
  -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 -o ServerAliveInterval=30
  -o ServerAliveCountMax=3 -p <ssh_port> -i <key> -R 0.0.0.0:<暴露端口>:127.0.0.1:<固定端口>
  <user>@<server>`。BatchMode=yes 禁止交互输入（鉴权只走私钥/ssh-agent）；ExitOnForwardFailure
  让转发绑定失败（端口占用/GatewayPorts 未开）直接退出并进 stderr；错误关键字（Permission
  denied/remote port forwarding failed 等）命中 → 计数退避重启，累计 MAX_FAILURES 后
  终态 Failed 并把错误透出；进程稳定存活 UP_TIMEOUT 且无错 → Up。停止走 kill 进程树 +
  Job Object 兜底。exe 经 `Platform::ssh_client_exe` 取（Windows 优先系统自带
  C:\Windows\System32\OpenSSH\ssh.exe，Win10 1809+ 自带；测试注入 node + fixture 脚本）。
- `proxy.rs` — axum 反向代理，绑 `0.0.0.0:<固定端口>`（局域网直接可达）。鉴权：有效 cookie `__dsh_remote` 直接转发；`?token=` 匹配（常数时间比较）→ 302 剥离 token + 种 HttpOnly cookie（**不带 Secure**：局域网直连是明文 HTTP，Secure cookie 浏览器在 http 下不存不发，带了整条鉴权链断掉）；token 不匹配 → 固定 500ms 延迟后 403；无凭据 → 403 门页。HTTP 经 reqwest 流式转发（3xx 透传不跟随）；WS upgrade 在代理终结握手后与 dsh 另建连接逐帧双向桥接。dsh 端口走 `watch::Receiver` 动态读取，dsh 重启代理不断线。**token 存共享单元（RwLock），门岗逐请求读最新值**——重置后旧链接/旧 cookie 即刻失效；**WS 桥接同时挂 drain Notify**（`enable()` 提前挂号防 connect 窗口期漏掐），重置/停服 `notify_waiters` 掐断所有已建立连接，否则泄露场景下攻击者已开的页面仍能持续收事件流。**转发必须剥掉浏览器标记头**（`origin`/`referer`/`sec-fetch-*`）：dsh 的 /api 信任栅栏（dsh-client-connection `isTrustedApiRequest`）要求 Origin.host == Host 头且拒绝 `sec-fetch-site: cross-site`，局域网/隧道场景 Origin 是 http://<局域网IP>:<端口>（或 trycloudflare）域名，不剥则页面所有 RPC 调用全 403；剥掉后请求在 dsh 眼里是无 Origin 的 loopback 客户端（WS 桥接侧 tungstenite 握手本就不带 Origin，天然满足）。**转发客户端必须 `.no_proxy()`**——用户系统代理（Clash 等）否则会把 127.0.0.1 转发劫持走。**插件 bundle 改写**：`/plugins/*/client.js` 响应被缓冲（≤4MB、仅 identity 编码）并把 `connection.isLoopback ? "host" : "memory"` 全量替换为 `"host"`——dsh 内测声明（WelcomeNoticeStore）对非回环源选 memory 持久化，局域网/隧道访问下确认记录不落 settings.yaml 导致每次连接都弹；改写后远程端与桌面端共用 host 持久化（桌面本就已确认，远程直接不弹）。改写路径转发时剥 `accept-encoding`（求 identity）与条件请求头（防 304），响应剥 `content-length`/`etag`；needle 失配（dsh 改版换写法）静默原样透传，声明照弹但不破坏页面。**HTML 文档注入移动端适配（mobile.css + mobile.js）**：文档导航请求（accept 含 text/html）且响应确为 `text/html` 时，缓冲后往 `</head>` 前注入 `mobile.css` 与 `mobile.js`（编译期 `include_str!` 内嵌，注入点带 `<!-- dshdesktop-mobile -->` 标记）——dsh Web UI 未做手机适配，实测三处破版：设置弹窗左侧 188px 固定导航列把内容区压到一字一行竖排（全屏化 + 导航改顶部横向 tab 条修复）、narrow 模式展开侧栏占 280px 固定网格轨把主区压到 110px（轨道归零 + 侧栏内容溢出成抽屉修复）、输入区 trailing 组 `flex:0 0 auto` 把 + 按钮压到与模型名重叠（模型选择器图标化：隐藏型号/推理档位文案、mask+currentColor 补火花图标，型号在点开的二级菜单里选；触发器改静态定位把弹出菜单的包含块上移到输入卡片，修 right:0 右对齐导致的左越界裁切；回合统计行 ~460px 宽单行必截断——mobile.js 随同注入（dsh 无 CSP）：在会话页“对话/轨迹”旁加“信息”标签，点开把统计行克隆进全屏面板逐行展示（MutationObserver 跟随同步；克隆而非搬家——React 对被移走的节点 removeChild 必崩），打 data-dshmobile-enhanced 后隐藏输入区下方的原行，且标记跟随 matchMedia 断点（旋屏/拉窗离开 700px 即摘除恢复原行，防宽屏下标签被 CSS 隐藏、统计无处可见）；JS 失效或节点未命中时 CSS 兜底：隐藏 | 分隔符改弹性换行两行居中，信息全保留）。断点 700px（桌面壳窗口最小宽 900px 永不命中，注入只影响经代理的远程访问）；选择器只锚语义钩子（`[role="dialog"]`、`data-sidebar-collapsed` 展开时属性不存在）与 CSS Modules 本地名子串（`[class*="_nav"]` 等，哈希前缀随版本变化不影响），上游改名则对应规则静默失效回到未适配状态。找不到 `</head>` 或非 HTML 一律原文透传。
（旧版曾有 `tunnel.rs`——cloudflared quick tunnel 监督；已删除，远程访问不再依赖外网隧道。） (feat: 远程访问改用局域网固定端口 + SSH 反向隧道内网穿透)

**安全模型**：链接即凭据（托盘/二维码页有"勿分享"提示）；token 每次开启重新生成，停止/退出应用即整体失效（quit 顺序：先 stop 远程访问再 stop dsh）；泄露时"重置链接"一键吊销（token 轮换 + 掐断现有会话，端口不变）。鉴权代理监听 `0.0.0.0:<端口>`，dsh 本身仍只监听 loopback。**token 不落日志**：events.log 只记 phase/url/error/proxy_port（不含链接），任何 `?token=` 查询串经 `redact_token` 脱敏；开启成功的 toast 正文不带链接（系统通知中心会留痕），只提示去托盘复制。**已知取舍**：链路是明文 HTTP（无 TLS），同一局域网内可被嗅探——靠"局域网信任 + 每次开启轮换 token + 泄露即重置"兜底，建议仅在可信网络（自家 Wi-Fi）使用；SSH 隧道把明文 HTTP 暴露到公网，暴露面更大，务必用强私钥、注意服务器端口防火墙放行范围；端口固定但 token 每次开启都换，链接仍不能收藏复用（每次开启重新扫码）。

**分发**：不再随 runtime 携带 cloudflared，`fetch-runtime.ps1` 已移除相关下载；远程访问零外部依赖（纯壳内 axum 代理 + 系统自带 OpenSSH 客户端，Win10 1809+ 无需额外安装）。

**UI**：托盘子菜单（开启/关闭互斥 enabled、复制链接、显示二维码、重置远程链接——复制/二维码/重置仅 Up 可用）+ `#/remote` 本地窗口（二维码 SVG 由 `qrcode` crate 生成、复制、重置（confirm 确认）、开关按钮）+ 诊断面板"远程访问"状态行 + "其它设置"的"远程访问端口"输入（默认 7788，保存即生效，下次开启用新端口）与"SSH 隧道（内网穿透）"卡片（开关/服务器地址/SSH 端口/用户名/私钥路径（原生文件选择）/暴露端口/链接端口（可选，留空跟随暴露端口，反向代理对外公布时填对外端口）；开启时前端与后端都校验必填与端口范围）。locale 切换重建托盘菜单后 `TrayRemoteItems` 句柄替换并按当前 phase 重设 enabled。Windows 防火墙首次开启时可能弹"允许访问"提示，需放行 DSHDesktop 与该端口。

**测试**：`tests/remote_proxy.rs`（门岗 403/302/种 cookie（含"无 Secure"断言）/转发/WS 桥接/503/shutdown 释放端口/插件 bundle 三元式改写/HTML 文档移动端样式与信息标签页脚本注入、无 </head> 透传）、`tests/remote_manager.rs`（端口占用报错、fixture dsh + 固定端口全链路 up→stop、改配置下次生效、重置轮换 token 端口不变、SSH 隧道 up 后链接用服务器地址、SSH 失败（fake-ssh 打 Permission denied）转 error 透出错误）、`remote/ssh_tunnel.rs` 单元测试锚定命令行形态（BatchMode/ExitOnForwardFailure/-R 0.0.0.0:port:…）。局域网真机与真实 SSH 服务器链路不进自动化，手动验收：局域网 → 托盘开启 → 手机连同一 Wi-Fi 扫码；SSH → 在设置填好服务器信息 → 托盘开启 → 异地浏览器打开 http://服务器:暴露端口。

## 17. 应用更新检查（update.rs）

"其它设置 → 检查更新"卡片：手动更新 + "启动时自动检查更新"开关（settings.json `check_update_on_launch`，默认关）。

**链路**：GitHub `releases/latest` API（`LBurny/deepseek-harness-desktop`）→ 比较 tag 与 `CARGO_PKG_VERSION` → 有新版则流式下载 `*_x64-setup.exe` 资产到系统下载目录（`.part` 写毕 rename，防半成品被当完整包）。版本比较自实现（去 `v` 前缀、`-rc`/`+build` 后缀忽略、逐段数值、短序列补 0；任一侧解析失败按"非新版"处理，宁漏报不误报），未引 semver crate。

**要点**：

- reqwest **走系统代理**（访问外网 GitHub，代理是通路必要条件；与 remote/proxy.rs 回环必须 `.no_proxy()` 正好相反）；GitHub API 必须带 User-Agent 否则 403
- 检查 15s 超时；下载只设 connect 超时不设总超时（60MB 慢网）
- 进度事件 `update-download-progress {downloaded,total}` 按百分比变化节流（同 lib.rs copy_cb），收尾强制 100%（content-length 与实际字节数可能不一致）
- 4 个命令（check_update/download_update/install_update/open_update_page）走既有 ACL 三处同步（build.rs/capabilities/default.json；dsh-remote 不开）；未引入 opener 插件——`open_update_page` 用 rundll32 `FileProtocolHandler`（GUI 子系统不闪控制台），`install_update` 校验路径以 `_x64-setup.exe` 结尾后 spawn，随后走 `quit_app` 让本进程先行退出（安装器是本进程子进程，旧版钩子的 `taskkill /T` 会连它一起杀；本进程先死，钩子杀树即成空操作），用户在向导里完成覆盖安装
- 启动时检查（开关开启时）在 setup 末尾 spawn：有新版弹 toast 指向其它设置页，失败只记 events.log
- 单元测试只覆盖纯函数（版本解析/比较、资产选择、响应反序列化容错）；真实网络链路不进自动化，手动验收：其它设置 → 手动更新 → 进度条 → 立即安装
## 18. 插件管理（plugins.rs）

dsh 的"插件"= 声明了 `dsh.bundle` 的 npm 包（cordis bundle，装进 profile 后作为层加载，UI 插件出现在 `/plugins/<id>/client.js`）。装/卸/更新**全部走 dsh 官方 `plugin` 子命令**（`node bin.js plugin --profile web <pnpm args>`）：profile 首次使用时由上游初始化，`pnpm add/remove/update` 在 `<dsh-home>/profiles/web/` 里跑完，上游按**安装态**对账 `dsh.profile.bundles` 层列表（解析到声明 `dsh.bundle` 的包就入层栈，被移除或新版丢声明的就踢出）。壳**不自己写 bundles**——对账逻辑归上游，跟版只动 upstream.rs 常量 + 契约测试。

**pnpm 壳内置**（`dsh plugin` 内部是 `spawnSync("pnpm", ...)`，Windows 上带 `shell: true` 走 cmd.exe 解析）：fetch-runtime.ps1 从 npm registry 下载 pnpm tarball，整包保留为 `<runtime>/pnpm/`（`bin/pnpm.cjs` → `./pnpm.mjs` → `../dist/pnpm.mjs`，dist 是 14MB standalone 全量 bundle，**不能摊平**），同时生成 `pnpm.cmd` 包装（调同目录 node.exe 跑 `pnpm\bin\pnpm.cjs`）——Windows 按 PATHEXT 只认 .exe/.cmd/.bat，没有 .cmd 包装 dsh 解析不到 pnpm。壳侧 spawn 时把 runtime 目录**前置到 PATH**（`.env("PATH", ...)` 全量保留原 PATH），dsh 内部即可解析到内置 pnpm，不污染用户环境。⚠️ 开发机测试时别用 Git Bash 手工验证 spawnSync 解析——msys 会把 `H:\...` 路径改写成 `H;C:\...` 伪失败；集成测试（cargo test，原生进程环境）是权威验证。

**命令与数据流**（6 个，PluginsHome 状态托管 node_exe/dsh_bin/home/pnpm_dir，与 DshProcess 同源路径）：

- `get_plugin_status`：pnpm 就绪态（`pnpm.cmd` + `pnpm\bin\pnpm.cjs` 都存在）+ `node pnpm.cjs --version` 实测版本 + profile 是否已初始化（`profiles/web/package.json` 存在）
- `list_plugins`：读清单 `dependencies`（版本）与 `dsh.profile.bundles`（标"插件"徽章，其余标"依赖"）；文件缺失 → 空列表；BOM 容忍；解析失败显式报错
- `search_plugins(q)`：npm registry search API（`registry.npmjs.org/-/v1/search`，reqwest 带 UA；外网走系统代理——与回环的 no_proxy 相反，同 update.rs 约定）；结果与已装列表交叉标"已安装"；查询 <2 字符直接返回空
- `install_plugin(spec)` / `uninstall_plugin(name)` / `update_plugins()`：`run_plugin_op` 统一执行——`node bin.js plugin --profile web <args>`，DSH_HOME 注入、PATH 前置、无 shell（参数直接走 argv，杜绝注入）、`configure_child_command`（CREATE_NO_WINDOW 防闪控制台）、spawn 后 `register_child` 挂全局 Job Object（壳被杀连带回收，防孤儿）；**stdout/stderr 必须显式 pipe**——tokio 的 spawn 默认继承父进程 stdio，`wait_with_output` 只读管道句柄，不接管道则 output 恒为空（前端"看下方输出"永远没内容，0.2.0 实踩）；输出 stdout+stderr 合并截断 200KB 返回。`validate_spec` 拦截空/超长/`-` 开头（防参数注入）。`busy` Mutex 串行锁：同一时刻只允许一个操作（try_lock 失败报"进行中"），防并发写 profile。

**IPC 契约**：本模块返回前端的结构体（`PluginOpResult`/`PluginStatus`/`PluginRow`）一律 `#[serde(rename_all = "camelCase")]`——前端按 camelCase 读键，漏了 rename 时多词字段（`exit_code`/`pnpm_ready`/`is_bundle`）在前端恒为 `undefined`：`exitCode === 0` 永不成立 → 成功被误报"失败"、pnpm 状态恒显示"缺失"、bundle 徽章恒显示"依赖"（0.2.0 全中）。`ipc_payloads_serialize_camel_case` 锚定测试守门。

**生效方式**：装/卸/更新**没有 MCP 那种 HMR**——新层经 profile manifest 在 web 启动时加载，完成后面板提示"重启 dsh 后生效"，内置"重启 dsh"按钮复用 `restart_dsh` 命令（装多个插件只需最后重启一次）。运行中安装不冲突（Node 模块文件句柄带共享删除标志，pnpm 增删无碍）；若 pnpm 报错引导先重启再重试。

**已知取舍**：registry 搜索请求本身不进自动化测试（同 update.rs 策略，只测解析）；集成测试（`tests/plugins_integration.rs`，无运行时自动 skip）用真实 dsh bin.js + 假 pnpm.cmd 断言 profile 初始化、参数透传、cwd=profile 目录、退出码透传、PATH 注入生效；真实安装链路手动验收（托盘 → 插件管理 → 搜索 → 安装）。契约测试 `probe_plugins_cli` 探测 bin.js 的 `command("plugin")` 与 `requiredOption("--profile <name>")`——上游改版即红。

## 19. 目录选择器钉 browse（picker.rs）

dsh 新建工作区要选文件夹，选择器有两套交互，启动时由 `directory-picker-auto` 一次性决议：绑 127.0.0.1 + win32 ⇒ **native**（koffi 驱动 Win32 系统对话框，弹在电脑屏幕上）；非回环/SSH ⇒ **browse**（网页内嵌对话框）。壳的远程代理对 dsh 透明，dsh 永远决议 native——**手机远程端点"添加工作区"，系统对话框弹在电脑屏幕上，手机上什么都看不到，无法选择**。

修复走上游官方 pin 方式（`apps/web/tests/pin-browse-picker.overlay.yml` 与 shipped bundle patch 行注释明示）：`picker::ensure_browse_picker` 在 spawn dsh 前往 `<dsh-home>/profiles/web/cordis.patch.yml` 幂等确保两条补丁——`{id: directory-picker, disabled: true}` 禁用 auto 行，insert `@deepseek-ai/dsh-host-directory-picker-browse`（host 列目录/建目录）+ `@deepseek-ai/dsh-client-ui-directory-picker-browse`（网页表面，占 ui-workspace 的 directory-flow 槽位）。与 mcp.rs 管理同一文件：mcp 只认 `name=='@deepseek-ai/dsh-mcp-client'` 的 insert 条目，picker 的三条互不命中，Value 级共存（read_patch/write_patch 复用 mcp.rs，BOM 容忍 + tmp+rename 原子写）；缺行补行、用户手加的 disabled 摘掉，只在有变化时写盘（无谓写会触发 HMR 重载）；失败只记 events.log 不阻断启动。

**桌面端同步变为网页版对话框**（选择器是 dsh 启动期全局决议，无法桌面 native/手机 browse 并存）——功能不减：浏览全盘、面包屑、手输路径（前辍过滤）、新建文件夹。对话框本体是上游 figma 设计（680×500 viewport-clamped，Miller 双栏窄屏横滚 + JS 自动钉右），mobile.css 只补布局：≤700px 时高度放宽到 `calc(100dvh - 48px)`（500px 上限在手机上列表仅 ~9 行），footer 三控件一行均分且移除"显示隐藏文件"开关（隐藏条目已由 pickerpatch 改为默认显示，开关在手机一行布局里挤占"新建文件夹"），锚点 `_millerRow` 是该包独有 CSS Modules 本地名（`:has` 限定不误伤设置弹窗）。

**pickerpatch.rs 运行时补丁**（presets.rs 同款签名门控 + marker 幂等原地改写，dsh 自更新还原后下次启动重打；needle 收口 upstream.rs、`probe_pickerpatch` 守门）：
- *盘符层级*：host `list()` 特判哨兵路径 `"dsh:drives"` 返回 A-Z 可用盘符根（不可读/未就绪的盘 stat 跳过），`ancestryCrumbs` 对盘符根路径前插"此电脑" crumb——没有这一层，面包屑在 home 子树内被客户端 `displayCrumbs` 折叠成单个"主页"，想到其它盘只能手输路径（手机端实踩痛点）。客户端配套：`displayCrumbs` 折叠时保留哨兵 crumb 居首（任意位置一键回盘符层）、哨兵 crumb 走 locale 文案（`browser.drives` 此电脑/This PC，host 不知道客户端语言）、哨兵层级禁用"打开/新建文件夹"（防把 `"dsh:drives"` 选成工作区/当父目录）。
- *隐藏条目默认显示*：client 的 `showHidden` 初值与每次开框重置都改 `true`。
- *耦合规则*：客户端是哨兵功能的安全前提（本地化 crumb + 禁用"打开"），其签名漂移/文件缺失时整组停手——只改 host 会放出能把哨兵选成工作区的半成品。

**跟版门禁**：`probe_picker` 契约探测——shipped bundle patch 仍含 `id: directory-picker` 的 auto 行（disable 目标）、两个 browse 包仍在依赖闭包（insert 行能被 Loader 解析）；`probe_pickerpatch` 核对两包内文件的全部补丁 needle（host 2 处 + client 8 处），上游改版即红；上游若默认 browse 即可删 picker.rs。`remote_proxy.rs` 的注入测试断言 `_millerRow` 规则随 mobile.css 注入。
