# DSHDesktop

[deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（dsh，DeepSeek agent harness CLI）的 Windows 桌面壳应用：Tauri 2 窗口内嵌 dsh 官方 Web UI，Node.js 与 dsh 随安装包分发、装完即用。

## 技术栈与形态

- **Tauri 2 + Rust**（`src-tauri/`）：进程监督、运行时部署、托盘、通知、主题跟随、诊断命令
- **Svelte 5 + TypeScript**（`src/`）：启动画面（splash）、诊断面板、其它设置三个本地页面；主界面是导航到的远程 dsh Web UI（`http://127.0.0.1:<port>`）
- 安装包：NSIS（`pnpm tauri build`），单实例、托盘常驻、关窗默认隐藏到托盘（可在"其它设置"改为直接退出）

## 目录结构

```
src-tauri/src/
  lib.rs            Builder 组装：插件(single_instance 必须最先；window-state 记忆窗口几何，
                    flags 不含 VISIBLE 防托盘隐藏态被记住；restore 晚于首批可见帧，
                    故主窗口 conf visible:false 创建、on_page_load(Finished) 再 show)
                    → setup → 事件桥(dsh-ready→导航)
  platform/         平台抽象 trait（多平台预留）；windows.rs 实现；macos/linux 待实现
  process.rs        DshProcess 监督循环：spawn node bin.js web --port N、指数退避、stop/restart
  runtime.rs        ensure_runtime：安装目录可写则原地运行内嵌运行时；只读则回退部署副本
                    （.version 比对）；原地模式会清理旧版留下的 %LOCALAPPDATA% 部署副本
  notify/           WS 事件源（ws.rs 泛化 {path, handler, on_connect}，连 events.mux +
                    events.host 双下行）+ 帧分类（approval/question、turn/end 完成、
                    session/title 入 SessionBook 台账；子代理经 host 流 origin 过滤）
  theme.rs          标题栏主题跟随：轮询 dsh-home/settings.yaml 的 ui-theme.preference；
                    首启播种（settings.yaml 缺失时按系统深浅色预写 preference，dsh 缺省是浅色）
  progress.rs       首启进度模型：阶段权重、百分比映射、结构化 dsh-progress 负载
  tray.rs           系统托盘菜单（打开/诊断/技能管理/MCP 管理/重启/其它设置/退出）；
                    diagnostics.rs 状态/日志环形缓冲；commands.rs 7 个 invoke 命令
  zoom.rs           UI 缩放：钩子脚本 hook_js(settings) 动态内嵌快捷键(on_page_load eval，
                    只注入 main 窗口)、direction 命令按设置读步进、ui-zoom.txt 持久化
  settings.rs       壳设置：settings.json 模型（步进 1-25%/快捷键/关窗行为/
                    完成通知开关/提示音 silent|default|im|mail|reminder|sms）、校验、
                    SettingsState、get/set_shell_settings + preview_completion_sound 命令
  skills.rs         技能管理：DSH_HOME(壳注入，非~/.dsh)的 skills/(启用) ↔
                    skills-disabled/(停用) 目录移动即开关（dsh watcher 热刷新）；
                    启动自动种子 ~/.dsh/skills(.skills-seeded marker 防复活)；
                    三源导入(codex/claude/opencode)、冲突覆盖/跳过、删除；
                    5 个命令 + SkillsHome 状态
  mcp.rs            MCP 管理：读写 dsh-home/profiles/web/cordis.patch.yml 中
                    name=='@deepseek-ai/dsh-mcp-client' 的 insert 条目（其余条目
                    Value 级保留，tmp+rename 原子写，BOM 容忍）；启停=entry 上
                    disabled:true（cordis loader 原生，HMR 热生效无需重启）；
                    编辑保留 toolCallTimeoutMs/reconnect 等高级键；启动种子
                    ~/.dsh 两层 patch（.mcp-seeded marker 防复活，源里 disabled
                    的不同步）；导入 claude(.claude.json)/codex(config.toml)/
                    opencode(opencode.json)，sse 不支持标记跳过；
                    6 个命令 + McpHome 状态
  remote/           远程访问：mod.rs=RemoteManager(生命周期/token/5 命令) +
                    proxy.rs(axum token 门岗反向代理，cookie 种发，HTTP 流式转发
                    + WS 帧桥接；转发必须剥 origin/referer/sec-fetch-* 浏览器标记头
                    （dsh /api 信任栅栏：Origin.host≠Host 头或 cross-site → 403）；
                    转发客户端必须 .no_proxy() 防系统代理劫持 127.0.0.1) +
                    tunnel.rs(cloudflared quick tunnel 监督，stdout 解析
                    trycloudflare URL，退避重启后域名变 token 不变)；
                    托盘子菜单开关/复制/二维码(#/remote 窗口)
src/                splash/Splash.svelte、diagnostics/Diagnostics.svelte、
                    settings/Settings.svelte、skills/Skills.svelte、
                    mcp/Mcp.svelte、remote/Remote.svelte、App.svelte(hash 路由)
scripts/            fetch-runtime.ps1(下载 Node+dsh+cloudflared+精简)、prune-runtime.ps1(精简运行时)、
                    acceptance.ps1(端到端验收)、shot-window.ps1(窗口截图)、
                    hide-show-theme.ps1(托盘隐藏回归)、get-attr20.ps1(读 DWM 深色属性)、
                    simulate-first-launch.ps1(模拟首启并截图)、verify-zoom.ps1(UI 缩放目验，需先 pnpm dev)、
                    verify-window-state.ps1(窗口几何记忆回归：调尺寸→退出→重启→断言恢复)、
                    verify-no-size-flash.ps1(尺寸闪变回归：预写状态文件→断言首个可见帧即记忆几何)、
                    verify-completion-notify.ps1(完成通知回归：fixture 运行时+隐藏窗口→断言
                    events.log 出现 Notify: TurnCompleted)、use-fixture-runtime.ps1、gen-icon.mjs
docs/design.zh-CN.md / design.md                  设计文档（架构/模块/打包/测试/已知限制，先读它）
```

## 常用命令

```bash
# 开发（需要 fixture 运行时：先跑 scripts/use-fixture-runtime.ps1，再设 DSHDESKTOP_RUNTIME_DIR）
cd src-tauri && cargo test            # 全部测试（112 个：单元+进程集成+WS通知+控制台窗口+远程访问）
pnpm tauri build                      # 产出 src-tauri/target/release/bundle/nsis/DSHDesktop_*_x64-setup.exe
powershell -File scripts/fetch-runtime.ps1   # 抓取真实运行时到 src-tauri/runtime/windows-x64/
powershell -File scripts/acceptance.ps1 -SetupExe <setup.exe>   # 卸载旧版→安装→启动→全项校验→截图
```

## 关键约定与坑（细节见 docs/design.zh-CN.md）

- **dsh 事实**：Node `^22.19 || >=24`；入口 `lib/bin.js`；`dsh web` 只许绑 127.0.0.1；事件走 **WebSocket** `/api/events.mux` + `/api/events.host`（GET 返回 426），帧格式 `{"type":"server-request","method":<payload.type>,"payload":{...}}`；完成判定看 `session/event` 里的 `turn/end`（`data.reason.kind=="completed"`），子代理标记看 `host/session-added` 的 `origin`；设置在 `$DSH_HOME/settings.yaml` 的 `ui-theme.preference`（light/dark/system）
- **运行时布局**：暂存 `src-tauri/runtime/<triplet>/`，tauri.conf `resources: ["runtime"]`，安装后 `<install>/runtime/<triplet>/`；`bundle.resources` 相对路径原样映射（`..` 会变 `_up_`，别用）
- **子进程控制台**：`Platform::configure_child_command` 设 CREATE_NO_WINDOW；验收判据是**可见 ConsoleWindowClass 窗口**（conhost 进程存在≠窗口可见）
- **PowerShell 5.1**：含中文的 .ps1 必须 UTF-8 **带 BOM**（注意 ZCode Edit 工具改完会丢 BOM，须补回）；别用 PS 改写 `settings.yaml`（会引入 BOM 导致 yaml-rust 解析失败，主题静默回退）
- **脚本里别用 Process.MainWindowHandle**：debug exe 还持有可见控制台与 Tao/托盘辅助窗口，句柄会指错；按 class "Tauri Window" 枚举进程顶层窗口（verify-no-size-flash.ps1 / verify-window-state.ps1 的 FindByClass 模式）
- **Tauri setup 无 tokio 上下文**：spawn_supervised 必须经 `tauri::async_runtime::block_on`
- **Tauri `resource_dir()` 返回 `\\?\` 扩展路径**：Node 加载器不认（EISDIR 崩溃），`runtime::strip_verbatim` 已处理，别绕过 ensure_runtime 自己拼路径
- **外部诊断手段**：`%LOCALAPPDATA%\DSHDesktop\events.log` 记录每个进程事件（1MB 截断），应用卡启动时先看它
- **fixture 用 .cjs**（根 package.json 是 type:module）；`#[tokio::test]` 涉及 std::thread::sleep 时须 `flavor="multi_thread"`。use-fixture-runtime.ps1 会在 @deepseek-ai/dsh 下铺 CJS 桩 package.json——fetch-runtime 抓过的树带真实 `"type":"module"`，不铺桩 mock bin.js 会按 ESM 加载崩溃
- **dev 模式 tauri 不拷贝 bundle.resources**：内置音效（resources/sounds/*.wav）在 dev 下要手动复制到 `src-tauri/target/debug/sounds/`，否则自定义提示音静默降级为系统默认；生产包由 bundle.resources 正常打进安装目录。另外真实运行时放 src-tauri/runtime 下跑 dev 会被 dsh 自更新（package-lock/node_modules 变动）触发 watcher 重建循环——复制到 src-tauri 外用 DSHDESKTOP_RUNTIME_DIR 指向
- **NSIS 离线**：github 直连不稳时用 ghproxy.net 预置 `%LOCALAPPDATA%\tauri\NSIS`（含 nsis_tauri_utils.dll，SHA1 须匹配 bundler 常量）
- **托盘 quit 顺序**：先 stop dsh 等 1.5s 再 exit；杀子进程树用 `taskkill /T /F`
- **远程 IPC 放行**：dsh UI 是远程源，远程 IPC 一律走 ACL。build.rs 用 `AppManifest::commands` 声明全部 22 个命令（生成 `permissions/autogenerated/allow-*.toml`），`capabilities/dsh-remote.json` 只对 `http://127.0.0.1:*` 开放 `allow-zoom-ui`；副作用是本地命令也全部 ACL 化——**新增命令要同步三处**：build.rs、capabilities/default.json、按需 dsh-remote.json
- **缩放快捷键匹配**：主匹配 `e.code`，`e.key` 兜底（合成按键/RDP 注入 keydown 的 `e.code` 为空）；zoom_ui 负载是 `direction:"in"/"out"`，步进由命令读设置（不写死在脚本里）；改快捷键须重注入钩子（set_shell_settings 已做，热替换不叠加）
- **reqwest 在系统代理下会劫持 127.0.0.1**：用户开 Clash 等系统代理时 reqwest 默认走代理且不认 bypass 列表——凡访问本机回环（remote/proxy.rs 转发客户端、测试里访问 fixture/代理端口的客户端）必须 `.no_proxy()`，否则请求被代理软件接管表现为假 502/挂起

## 测试基线

`cargo test` 应全绿（当前 112 个）。`tests/console_window.rs` 的对照组会在屏幕上短暂弹出真实控制台窗口，属正常。改主题/进程/通知逻辑后，跑 `cargo test` + 重装走一遍 `acceptance.ps1`。

## 多平台预留

平台差异都收口在 `platform/mod.rs` 的 `Platform` trait（节点可执行名、运行时目录、triplet、杀进程树、子进程配置、系统深浅色）。CI matrix 里 macos/linux 行已注释，启用前需实现对应 `platform/{macos,linux}.rs` 并在 fetch-runtime 支持对应 triplet。

## 已知限制

- Win10 深色标题栏聚焦时纯黑（系统行为，`DWMWA_CAPTION_COLOR` 仅 Win11）；要做成恒为 dsh 深灰需无边框自绘标题栏——方案要点见 docs/design.zh-CN.md §8，暂缓。
