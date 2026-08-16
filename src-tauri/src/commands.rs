use crate::diagnostics::{BootstrapInfo, SharedState, StatusDto};
use crate::theme::ShellUiState;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

/// 本地页面的主题/语言快照：页面加载即取，之后靠 shell-ui-state 事件增量更新
#[tauri::command]
pub fn get_shell_ui_state(state: State<ShellUiState>) -> crate::theme::UiSnapshot {
    state.get()
}

#[tauri::command]
pub fn get_bootstrap_error(state: State<BootstrapInfo>) -> Option<String> {
    state.error()
}

#[tauri::command]
pub fn is_first_launch(state: State<SharedState>) -> bool {
    state.first_launch
}

#[tauri::command]
pub fn get_status(state: State<SharedState>) -> StatusDto {
    StatusDto {
        state: format!("{:?}", state.process.state()),
        port: state.process.port(),
        pid: state.process.pid(),
        version: state.version.clone(),
    }
}

#[tauri::command]
pub async fn restart_dsh(state: State<'_, SharedState>) -> Result<(), String> {
    state.process.restart().await;
    Ok(())
}

#[tauri::command]
pub fn get_recent_logs(state: State<SharedState>) -> Vec<String> {
    state.log_ring.snapshot()
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let m = app.autolaunch();
    let r = if enabled { m.enable() } else { m.disable() };
    r.map_err(|e| e.to_string())
}
