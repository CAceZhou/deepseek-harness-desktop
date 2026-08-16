use crate::diagnostics::SharedState;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MENU_OPEN: &str = "open";
pub const MENU_DIAGNOSTICS: &str = "diagnostics";
pub const MENU_RESTART: &str = "restart";
pub const MENU_ZOOM_IN: &str = "zoom_in";
pub const MENU_ZOOM_OUT: &str = "zoom_out";
pub const MENU_ZOOM_RESET: &str = "zoom_reset";
pub const MENU_QUIT: &str = "quit";

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "打开主界面", true, None::<&str>)?;
    let diagnostics = MenuItem::with_id(app, MENU_DIAGNOSTICS, "诊断面板", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, MENU_RESTART, "重启服务", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let zoom_in = MenuItem::with_id(app, MENU_ZOOM_IN, "放大界面（Ctrl+Shift+=）", true, None::<&str>)?;
    let zoom_out = MenuItem::with_id(app, MENU_ZOOM_OUT, "缩小界面（Ctrl+Shift+-）", true, None::<&str>)?;
    let zoom_reset = MenuItem::with_id(app, MENU_ZOOM_RESET, "重置缩放", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &diagnostics, &restart, &sep, &zoom_in, &zoom_out, &zoom_reset, &quit],
    )?;

    TrayIconBuilder::with_id("main-tray")
        .tooltip("DSHDesktop")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main(app),
            MENU_DIAGNOSTICS => open_diagnostics(app),
            MENU_RESTART => {
                if let Some(state) = app.try_state::<SharedState>() {
                    let proc = state.process.clone();
                    tauri::async_runtime::spawn(async move {
                        proc.restart().await;
                    });
                }
            }
            MENU_QUIT => quit_app(app),
            MENU_ZOOM_IN => tray_zoom(app, Some(crate::zoom::ZOOM_STEP)),
            MENU_ZOOM_OUT => tray_zoom(app, Some(-crate::zoom::ZOOM_STEP)),
            MENU_ZOOM_RESET => tray_zoom(app, None),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 托盘触发的缩放：先呼出主窗口让用户看到效果，再改缩放。
/// delta 为 None 表示重置到 100%。
fn tray_zoom(app: &AppHandle, delta: Option<f64>) {
    show_main(app);
    if let Some(state) = app.try_state::<crate::zoom::ZoomState>() {
        let v = match delta {
            Some(d) => state.adjust(d),
            None => state.set(1.0),
        };
        crate::zoom::apply_to_main(app, v);
    }
}

fn open_diagnostics(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("diagnostics") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "diagnostics", WebviewUrl::App("index.html#/diagnostics".into()))
        .title("DSHDesktop 诊断面板")
        .inner_size(900.0, 640.0)
        .build();
}

fn quit_app(app: &AppHandle) {
    let Some(state) = app.try_state::<SharedState>() else {
        app.exit(0);
        return;
    };
    let proc = state.process.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        proc.stop().await;
        // 给监督循环留出杀进程树的时间，超时兜底强制退出
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        handle.exit(0);
    });
}
