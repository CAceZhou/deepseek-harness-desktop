use crate::platform::Platform;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, Theme};

/// 跟随 dsh 的主题设置（$DSH_HOME/settings.yaml 的 ui-theme.preference），
/// 同步所有窗口标题栏的深浅色；preference=system 时解析平台系统主题。
/// 采用 2s 轮询：文件极小、改动极少，轮询比 inotify 简单且跨平台无差异。
pub fn spawn_theme_follower(app: &AppHandle, platform: Arc<dyn Platform>, dsh_home: PathBuf) {
    let initial = system_theme(platform.as_ref());
    apply(app, initial);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = dsh_home.join("settings.yaml");
        loop {
            let resolved = resolve(platform.as_ref(), &settings);
            apply(&app, resolved);
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

fn system_theme(platform: &dyn Platform) -> Theme {
    if platform.system_dark_mode() {
        Theme::Dark
    } else {
        Theme::Light
    }
}

/// 首启播种：settings.yaml 不存在时按系统深浅色预写 ui-theme.preference。
/// dsh 缺省（无 preference）渲染浅色 UI，而壳标题栏缺省跟随系统；系统为深色时
/// 首启会出现"深标题栏 + 浅内容"的不一致。播种后 dsh 首启即与壳一致。
/// 注意：yaml-rust 不接受 UTF-8 BOM，std::fs::write 不会写 BOM。
pub fn seed_theme_preference(home: &Path, dark: bool) {
    let path = home.join("settings.yaml");
    if path.exists() {
        return;
    }
    let pref = if dark { "dark" } else { "light" };
    let _ = std::fs::create_dir_all(home)
        .and_then(|_| std::fs::write(&path, format!("ui-theme:\n  preference: {pref}\n")));
}

fn resolve(platform: &dyn Platform, settings: &Path) -> Theme {
    match read_theme_preference(settings).as_deref() {
        Some("dark") => Theme::Dark,
        Some("light") => Theme::Light,
        _ => system_theme(platform),
    }
}

fn apply(app: &AppHandle, theme: Theme) {
    #[cfg(windows)]
    apply_windows(app, theme);
    #[cfg(not(windows))]
    {
        for label in ["main", "diagnostics", "settings"] {
            if let Some(w) = app.get_webview_window(label) {
                let _ = w.set_theme(Some(theme));
            }
        }
    }
}

/// Windows 上双管齐下：
/// 1) set_theme 同步 tao 内部主题状态——否则 tao 可能在窗口事件（显示/聚焦）后用
///    缓存的旧状态覆盖可视效果；隐藏窗口上 set_theme 可能报错甚至 panic，须兜住。
/// 2) 直接对 HWND 设置 DWMWA_USE_IMMERSIVE_DARK_MODE——无缓存、幂等，对隐藏窗口
///    同样生效，是标题栏颜色的权威来源。
#[cfg(windows)]
fn apply_windows(app: &AppHandle, theme: Theme) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY: u32 = 19; // Win10 20H1 之前
    let value: i32 = matches!(theme, Theme::Dark) as i32;
    for label in ["main", "diagnostics", "settings"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = w.set_theme(Some(theme));
            }));
            if let Ok(hwnd) = w.hwnd() {
                let hwnd = hwnd.0 as _;
                unsafe {
                    let hr = DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_USE_IMMERSIVE_DARK_MODE,
                        &value as *const i32 as *const _,
                        std::mem::size_of::<i32>() as u32,
                    );
                    if hr != 0 {
                        DwmSetWindowAttribute(
                            hwnd,
                            DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY,
                            &value as *const i32 as *const _,
                            std::mem::size_of::<i32>() as u32,
                        );
                    }
                }
            }
        }
    }
}

fn read_theme_preference(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    // 配置文件可能被外部工具（如 PowerShell Set-Content -Encoding utf8）改写成
    // 带 UTF-8 BOM；yaml-rust 不接受 BOM，须先剥掉
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    let value: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    value
        .get("ui-theme")?
        .get("preference")?
        .as_str()
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_theme_preference() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("settings.yaml");
        std::fs::write(
            &f,
            "ui-onboarding:\n  welcomeNoticeVersion: 1\nui-theme:\n  preference: dark\n",
        )
        .unwrap();
        assert_eq!(read_theme_preference(&f).as_deref(), Some("dark"));
        std::fs::write(&f, "ui-theme:\n  preference: light\n").unwrap();
        assert_eq!(read_theme_preference(&f).as_deref(), Some("light"));
    }

    #[test]
    fn reads_theme_preference_with_bom() {
        // dsh 写的是无 BOM UTF-8，但外部编辑工具（PowerShell utf8）会加 BOM
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("settings.yaml");
        std::fs::write(&f, "\u{FEFF}ui-theme:\n  preference: dark\n").unwrap();
        assert_eq!(read_theme_preference(&f).as_deref(), Some("dark"));
    }

    #[test]
    fn missing_file_or_field_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_theme_preference(&dir.path().join("none.yaml")), None);
        let f = dir.path().join("settings.yaml");
        std::fs::write(&f, "other:\n  x: 1\n").unwrap();
        assert_eq!(read_theme_preference(&f), None);
    }

    #[test]
    fn seed_creates_only_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("dsh-home");
        // 不存在 → 按系统深浅色播种
        seed_theme_preference(&home, true);
        let text = std::fs::read_to_string(home.join("settings.yaml")).unwrap();
        assert!(text.starts_with("ui-theme:\n  preference: dark"));
        assert!(!text.starts_with('\u{FEFF}')); // 无 BOM
        // 已存在 → 不动（哪怕内容里没有 preference）
        std::fs::write(home.join("settings.yaml"), "custom: 1\n").unwrap();
        seed_theme_preference(&home, false);
        assert_eq!(std::fs::read_to_string(home.join("settings.yaml")).unwrap(), "custom: 1\n");
    }
}
