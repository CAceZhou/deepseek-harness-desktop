use crate::platform::Platform;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Theme};

/// 壳界面状态快照：解析后的主题（dark/light）与语言（zh/en）。
/// 经 `get_shell_ui_state` 命令与 `shell-ui-state` 事件同步给本地页面。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UiSnapshot {
    pub theme: String,
    pub locale: String,
}

pub struct ShellUiState(pub std::sync::Mutex<UiSnapshot>);

impl ShellUiState {
    /// 启动时的初始解析：settings.yaml 可能尚不存在（首启），
    /// 此时按系统解析；关注循环 2s 后会再校正并广播。
    pub fn new(platform: &dyn Platform, dsh_home: &Path) -> Self {
        let settings = dsh_home.join(crate::upstream::SETTINGS_FILE);
        let locale = resolve_locale(platform, &settings);
        // 立即写入全局语言：托盘菜单/窗口标题/通知等先按解析值渲染，
        // 首轮轮询（force 同步）再校正——若设置里是 en，等 2s 后才英文也可接受
        crate::i18n::set_locale(&locale);
        Self(std::sync::Mutex::new(UiSnapshot {
            theme: theme_name(resolve(platform, &settings)).to_string(),
            locale,
        }))
    }

    pub fn get(&self) -> UiSnapshot {
        self.0.lock().unwrap().clone()
    }
}

/// 跟随 dsh 的主题与语言设置（$DSH_HOME/settings.yaml 的
/// ui-theme.preference / locale.preference），同步所有窗口标题栏深浅色、
/// 托盘菜单深浅色，并向本地页面广播 shell-ui-state。
/// preference=system（或缺省）时主题解析平台系统主题、语言解析系统 UI 语言。
/// 采用 2s 轮询：文件极小、改动极少，轮询比 inotify 简单且跨平台无差异。
pub fn spawn_theme_follower(app: &AppHandle, platform: Arc<dyn Platform>, dsh_home: PathBuf) {
    let initial = system_theme(platform.as_ref());
    apply(app, initial);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = dsh_home.join(crate::upstream::SETTINGS_FILE);
        // 首轮强制同步：即便与启动快照相同，也要广播一次、并让托盘菜单按
        // 最终解析的语言重建（tray 在 ShellUiState 之前创建，可能是旧语言）
        let first = UiSnapshot {
            theme: theme_name(resolve(platform.as_ref(), &settings)).to_string(),
            locale: resolve_locale(platform.as_ref(), &settings),
        };
        sync_ui_snapshot(&app, first, true);
        loop {
            let resolved = resolve(platform.as_ref(), &settings);
            apply(&app, resolved);
            sync_ui_snapshot(
                &app,
                UiSnapshot {
                    theme: theme_name(resolved).to_string(),
                    locale: resolve_locale(platform.as_ref(), &settings),
                },
                false,
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

/// 快照有变化（或 force）才落库并广播；locale 变化额外重建托盘菜单与本地窗口标题。
fn sync_ui_snapshot(app: &AppHandle, snap: UiSnapshot, force: bool) {
    let Some(state) = app.try_state::<ShellUiState>() else {
        return;
    };
    let old = {
        let mut guard = state.0.lock().unwrap();
        if !force && *guard == snap {
            return;
        }
        std::mem::replace(&mut *guard, snap.clone())
    };
    crate::i18n::set_locale(&snap.locale);
    let _ = app.emit("shell-ui-state", &snap);
    if force || old.locale != snap.locale {
        crate::tray::apply_locale(app, &snap.locale);
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "dark",
        _ => "light",
    }
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
    let path = home.join(crate::upstream::SETTINGS_FILE);
    if path.exists() {
        return;
    }
    let pref = if dark { "dark" } else { "light" };
    let _ = std::fs::create_dir_all(home).and_then(|_| {
        std::fs::write(
            &path,
            format!(
                "{}:\n  {}: {pref}\n",
                crate::upstream::KEY_UI_THEME,
                crate::upstream::KEY_PREFERENCE
            ),
        )
    });
}

fn resolve(platform: &dyn Platform, settings: &Path) -> Theme {
    match read_theme_preference(settings).as_deref() {
        Some("dark") => Theme::Dark,
        Some("light") => Theme::Light,
        _ => system_theme(platform),
    }
}

/// dsh 语言：locale.preference ∈ {zh,en}；缺省跟随系统 UI 语言
/// （dsh 侧缺省是"跟随浏览器"，WebView2 的浏览器语言同样来自系统）。
fn resolve_locale(platform: &dyn Platform, settings: &Path) -> String {
    match read_locale_preference(settings).as_deref() {
        Some("en") => "en".to_string(),
        Some("zh") => "zh".to_string(),
        _ => {
            if platform.system_prefers_chinese() {
                "zh".to_string()
            } else {
                "en".to_string()
            }
        }
    }
}

fn apply(app: &AppHandle, theme: Theme) {
    #[cfg(windows)]
    apply_windows(app, theme);
    #[cfg(not(windows))]
    {
        // 遍历所有窗口而非写死 label——新增本地窗口（如 skills）自动跟随
        for w in app.webview_windows().values() {
            let _ = w.set_theme(Some(theme));
        }
    }
}

/// Windows 上双管齐下：
/// 1) set_theme 同步 tao 内部主题状态——否则 tao 可能在窗口事件（显示/聚焦）后用
///    缓存的旧状态覆盖可视效果；隐藏窗口上 set_theme 可能报错甚至 panic，须兜住。
/// 2) 直接对 HWND 设置 DWMWA_USE_IMMERSIVE_DARK_MODE——无缓存、幂等，对隐藏窗口
///    同样生效，是标题栏颜色的权威来源。
/// 另对进程设 PreferredAppMode（uxtheme 未文档化 API，tao 同源做法），
/// 让托盘右键菜单按解析后的主题绘制，FlushMenuThemes 立即刷新已建菜单。
#[cfg(windows)]
fn apply_windows(app: &AppHandle, theme: Theme) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY: u32 = 19; // Win10 20H1 之前
    let dark = matches!(theme, Theme::Dark);
    let value: i32 = dark as i32;
    // 遍历所有窗口而非写死 label——新增本地窗口（如 skills）自动跟随
    for w in app.webview_windows().values() {
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
    set_menu_app_mode(dark);
}

/// 托盘菜单深浅色：PreferredAppMode 2=ForceDark / 3=ForceLight（1903+）。
/// 不用 AllowDark——它跟随系统而非 dsh 主题。uxtheme 常年驻留进程，无需 FreeLibrary。
#[cfg(windows)]
fn set_menu_app_mode(dark: bool) {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    unsafe {
        let uxtheme = LoadLibraryA(b"uxtheme.dll\0".as_ptr());
        if uxtheme.is_null() {
            return;
        }
        // MAKEINTRESOURCEA(135) = SetPreferredAppMode，136 = FlushMenuThemes
        let set_mode = GetProcAddress(uxtheme, 135usize as *const u8);
        if let Some(f) = set_mode {
            let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            f(if dark { 2 } else { 3 });
            if let Some(fl) = GetProcAddress(uxtheme, 136usize as *const u8) {
                let fl: unsafe extern "system" fn() = std::mem::transmute(fl);
                fl();
            }
        }
    }
}

/// 读取 settings.yaml 里 `<section>.preference`；文件损坏/缺字段返回 None。
/// 配置文件可能被外部工具（如 PowerShell Set-Content -Encoding utf8）改写成
/// 带 UTF-8 BOM；yaml-rust 不接受 BOM，须先剥掉。
fn read_preference(path: &Path, section: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    let value: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    value
        .get(section)?
        .get(crate::upstream::KEY_PREFERENCE)?
        .as_str()
        .map(|s| s.trim().to_string())
}

fn read_theme_preference(path: &Path) -> Option<String> {
    read_preference(path, crate::upstream::KEY_UI_THEME)
}

fn read_locale_preference(path: &Path) -> Option<String> {
    read_preference(path, crate::upstream::KEY_LOCALE)
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
    fn reads_locale_preference() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("settings.yaml");
        assert_eq!(read_locale_preference(&f), None);
        std::fs::write(&f, "locale:\n  preference: en\n").unwrap();
        assert_eq!(read_locale_preference(&f).as_deref(), Some("en"));
        std::fs::write(&f, "\u{FEFF}locale:\n  preference: zh\n").unwrap();
        assert_eq!(read_locale_preference(&f).as_deref(), Some("zh"));
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
