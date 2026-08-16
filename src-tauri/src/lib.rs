use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, Url};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::watch;

pub mod commands;
pub mod diagnostics;
pub mod notify;
pub mod platform;
pub mod port;
pub mod process;
pub mod progress;
pub mod runtime;
pub mod settings;
pub mod theme;
pub mod tray;
pub mod zoom;

use notify::{Notification, NotifySink, NotifySource};
use process::{DshState, ProcessEvent};
use progress::ProgressPayload;

pub fn run() {
    tauri::Builder::default()
        // 单实例必须最先注册；第二次启动时聚焦已有主窗口
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::restart_dsh,
            commands::get_recent_logs,
            commands::get_autostart,
            commands::set_autostart,
            commands::get_bootstrap_error,
            commands::is_first_launch,
            zoom::zoom_ui,
            settings::get_shell_settings,
            settings::set_shell_settings,
        ])
        .on_page_load(|webview, payload| {
            if !matches!(
                payload.event(),
                tauri::webview::PageLoadEvent::Finished
            ) {
                return;
            }
            // 缩放钩子只注入主窗口（splash 与远程 dsh UI 两个阶段的同一窗口）；
            // 诊断/设置窗口不注入——否则设置窗口里录制 Ctrl+Shift+= 会先被钩子拦截
            if webview.label() == "main" {
                // 每次整页加载后重注入（SPA 内导航不重载页面，不会重复触发）。
                // 钩子内嵌当前快捷键设置；manage 之前的首帧用默认设置兜底
                let settings = webview
                    .app_handle()
                    .try_state::<settings::SettingsState>()
                    .map(|s| s.get())
                    .unwrap_or_default();
                let _ = webview.eval(zoom::hook_js(&settings));
                // 启动首帧补应用持久化缩放；也兜住 WebView2 重建后 zoom 丢失
                if let Some(state) = webview.app_handle().try_state::<zoom::ZoomState>() {
                    let _ = webview.set_zoom(state.get());
                }
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            tray::setup_tray(&handle)?;
            handle.manage(diagnostics::BootstrapInfo::default());
            let platform: Arc<dyn platform::Platform> = platform::current().into();
            handle.manage(zoom::ZoomState::new(platform.runtime_base_dir()));
            handle.manage(settings::SettingsState::new(platform.runtime_base_dir()));
            let version = app.package_info().version.to_string();
            let home_url = app
                .get_webview_window("main")
                .and_then(|w| w.url().ok())
                .unwrap_or_else(|| Url::parse("http://tauri.localhost/").unwrap());

            // dsh 就绪端口通道：Ready（含重启后）时更新，通知 SSE 适配器
            let (port_tx, port_rx) = watch::channel::<Option<u16>>(None);
            let sink_handle = handle.clone();
            let sink: NotifySink = Arc::new(move |n: Notification| {
                // 只在主窗口隐藏（托盘态）时弹原生通知，避免打扰正在操作的用户
                let visible = sink_handle
                    .get_webview_window("main")
                    .map(|w| w.is_visible().unwrap_or(true))
                    .unwrap_or(true);
                if !visible {
                    let _ = sink_handle
                        .notification()
                        .builder()
                        .title(n.title)
                        .body(n.body)
                        .show();
                }
            });
            let ws = Box::new(notify::ws::WsSource {
                filter: notify::EventFilter::default(),
            });
            tauri::async_runtime::spawn(ws.run(sink, port_rx));

            let source = std::env::var_os("DSHDESKTOP_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    platform.resource_runtime_dir(&app.path().resource_dir().unwrap_or_default())
                });

            // 首启判定：ensure_runtime 首次运行才会创建 dsh-home；此前不存在即首启
            let first_launch = !platform.runtime_base_dir().join("dsh-home").exists();

            let _ = handle.emit(
                "dsh-progress",
                ProgressPayload::new("runtime", "正在准备运行时…", Some(0)),
            );

            // 回退部署（只读安装目录）时按字节进度 emit；节流：百分比变化才发，
            // 避免复制数千个小文件时刷爆 IPC
            let deployed = Arc::new(AtomicBool::new(false));
            let last_pct = Arc::new(AtomicU8::new(0));
            let dep = deployed.clone();
            let lp = last_pct.clone();
            let copy_emit = handle.clone();
            let copy_cb = move |copied: u64, total: u64| {
                dep.store(true, Ordering::SeqCst);
                let pct = progress::copy_percent(copied, total);
                if pct != lp.swap(pct, Ordering::SeqCst) {
                    let _ = copy_emit.emit(
                        "dsh-progress",
                        ProgressPayload::new(
                            "runtime",
                            "正在部署运行时（仅首次安装需要复制依赖）…",
                            Some(pct),
                        ),
                    );
                }
            };
            let paths = match runtime::ensure_runtime(
                platform.as_ref(),
                &source,
                &version,
                Some(&copy_cb),
            ) {
                Ok(p) => p,
                Err(e) => {
                    // 不退出：记录错误供启动画面查询，窗口停在启动画面
                    let msg = format!("运行时就绪失败：{e}");
                    handle.state::<diagnostics::BootstrapInfo>().set_error(msg.clone());
                    let _ = handle.emit(
                        "dsh-progress",
                        ProgressPayload::new("error", msg, None),
                    );
                    return Ok(());
                }
            };
            let deployed = deployed.load(Ordering::SeqCst);

            let log_ring = diagnostics::LogRing::default();
            // 首启播种主题：settings.yaml 不存在时按系统深浅色预写 ui-theme.preference，
            // 否则 dsh 缺省渲染浅色而壳标题栏跟随系统（深色时不一致）。
            // 必须在 spawn_supervised 之前，dsh 首次启动即读到。
            theme::seed_theme_preference(&paths.home, platform.system_dark_mode());
            theme::spawn_theme_follower(&handle, platform.clone(), paths.home.clone());
            let emit_handle = handle.clone();
            let nav_home = home_url.clone();
            // 事件调试日志：诊断面板之外的最后手段（面板本身依赖应用内交互才能看到）
            let debug_log = paths
                .home
                .parent()
                .map(|p| p.join("events.log"))
                .unwrap_or_else(|| PathBuf::from("events.log"));
            // block_on 提供 tokio runtime 上下文，spawn_supervised 内部的 tokio::spawn 依赖它
            let proc = tauri::async_runtime::block_on(async {
                process::DshProcess::spawn_supervised(
                    platform.clone(),
                    paths.clone(),
                    log_ring.clone(),
                    move |event| {
                        append_debug_log(&debug_log, &event);
                        bridge_event(&emit_handle, &nav_home, &port_tx, deployed, event);
                    },
                )
            });
            handle.manage(diagnostics::SharedState {
                process: proc,
                log_ring,
                runtime: paths,
                version,
                home_url,
                first_launch,
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口关窗行为由设置决定：默认最小化到托盘（保持后台运行），
            // 也可配置为直接退出程序；诊断/设置窗口关窗 = 销毁
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let quit = window
                        .app_handle()
                        .try_state::<settings::SettingsState>()
                        .map(|s| matches!(s.get().close_behavior, settings::CloseBehavior::Quit))
                        .unwrap_or(false);
                    if quit {
                        tray::quit_app(window.app_handle());
                    } else {
                        let _ = window.hide();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running DSHDesktop");
}

/// 追加事件到调试日志；超过 1MB 时截断重来（只用于现场诊断，不求完备）。
fn append_debug_log(path: &PathBuf, event: &ProcessEvent) {
    use std::io::Write;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > 1024 * 1024 {
            let _ = std::fs::remove_file(path);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let line = match event {
            ProcessEvent::StateChanged(s) => format!("{s:?}"),
            ProcessEvent::Log(l) => l.clone(),
        };
        let _ = writeln!(f, "{line}");
    }
}

fn bridge_event(
    handle: &tauri::AppHandle,
    home_url: &Url,
    port_tx: &watch::Sender<Option<u16>>,
    deployed: bool,
    event: ProcessEvent,
) {
    match event {
        ProcessEvent::Log(line) => {
            let _ = handle.emit("dsh-log", line);
        }
        ProcessEvent::StateChanged(state) => match state {
            DshState::Starting => {
                let _ = handle.emit(
                    "dsh-progress",
                    ProgressPayload::new(
                        "starting",
                        "正在启动 dsh 服务…",
                        Some(progress::starting_percent(deployed)),
                    ),
                );
            }
            DshState::Ready { port } => {
                let _ = port_tx.send(Some(port));
                let _ = handle.emit("dsh-ready", serde_json::json!({ "port": port }));
                let _ = handle.emit(
                    "dsh-progress",
                    ProgressPayload::new("ready", "正在打开界面…", Some(100)),
                );
                if let Some(w) = handle.get_webview_window("main") {
                    if let Ok(url) = Url::parse(&format!("http://127.0.0.1:{port}/")) {
                        let _ = w.navigate(url);
                    }
                }
            }
            DshState::Failed(msg) => {
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.navigate(home_url.clone());
                }
                let _ = handle.emit(
                    "dsh-progress",
                    ProgressPayload::new("error", format!("启动失败：{msg}"), None),
                );
            }
            DshState::Stopped => {
                let _ = handle.emit(
                    "dsh-progress",
                    ProgressPayload::new("stopped", "dsh 服务已停止", None),
                );
            }
        },
    }
}
