use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

pub const STEP_MIN: f64 = 0.01;
pub const STEP_MAX: f64 = 0.25;
const FILE_NAME: &str = "settings.json";

fn step_default() -> f64 {
    0.02
}

/// 快捷键：code 为主（物理键位，真实键盘），key 兜底（合成按键/RDP 注入时 code 为空）。
/// 两者在录制时同时从同一个 keydown 事件取得。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub code: String,
    pub key: String,
}

impl Shortcut {
    pub fn matches(&self, code: &str, key: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        if self.ctrl != ctrl || self.shift != shift || self.alt != alt {
            return false;
        }
        (!code.is_empty() && code == self.code) || (!key.is_empty() && key == self.key)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    Background,
    Quit,
}

/// 任务完成通知的提示音。前 5 项直接透传 toast 的音频预设
/// （tauri-winrt-notification Sound::from_str → ms-winsoundevent:Notification.*，
/// Windows 系统内置，不依赖用户声音方案）；Silent = 不传 sound，toast 静音。
/// Chime/Drop/Mellow 是壳内置的柔和合成音（resources/sounds/*.wav）：
/// toast 静音，由壳用 PlaySoundW 异步播放（见 platform::Platform::play_sound_file）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompletionSound {
    Silent,
    #[default]
    Default,
    Im,
    Mail,
    Reminder,
    Sms,
    Chime,
    Drop,
    Mellow,
}

impl CompletionSound {
    /// tauri-plugin-notification builder.sound() 的取值；None 表示静音 toast
    /// （自定义柔和音也是静音 toast，声音由壳单独播放）
    pub fn toast_sound_name(self) -> Option<&'static str> {
        match self {
            CompletionSound::Silent => None,
            CompletionSound::Default => Some("Default"),
            CompletionSound::Im => Some("IM"),
            CompletionSound::Mail => Some("Mail"),
            CompletionSound::Reminder => Some("Reminder"),
            CompletionSound::Sms => Some("SMS"),
            CompletionSound::Chime | CompletionSound::Drop | CompletionSound::Mellow => None,
        }
    }

    /// 自定义柔和音的内置 wav 资源相对路径（相对 resource_dir）；None = 非自定义音
    pub fn custom_wav(self) -> Option<&'static str> {
        match self {
            CompletionSound::Chime => Some("sounds/chime.wav"),
            CompletionSound::Drop => Some("sounds/drop.wav"),
            CompletionSound::Mellow => Some("sounds/mellow.wav"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellSettings {
    pub zoom_step: f64,
    pub zoom_in: Shortcut,
    pub zoom_out: Shortcut,
    pub close_behavior: CloseBehavior,
    pub notify_on_completion: bool,
    pub completion_sound: CompletionSound,
}

impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            zoom_step: step_default(),
            zoom_in: Shortcut {
                ctrl: true,
                shift: true,
                alt: false,
                code: "Equal".into(),
                key: "+".into(),
            },
            zoom_out: Shortcut {
                ctrl: true,
                shift: true,
                alt: false,
                code: "Minus".into(),
                key: "_".into(),
            },
            close_behavior: CloseBehavior::Background,
            notify_on_completion: true,
            completion_sound: CompletionSound::Default,
        }
    }
}

fn file_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

impl ShellSettings {
    /// 读取设置；文件缺失/损坏 → 全默认；部分字段缺失 → 逐字段回退默认（serde default）。
    /// 步进越界在加载时 clamp，保证内存中始终合法。
    pub fn load(dir: &Path) -> Self {
        let mut s: Self = std::fs::read_to_string(file_path(dir))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        s.zoom_step = s.zoom_step.clamp(STEP_MIN, STEP_MAX);
        if s.validate().is_err() {
            // 配置文件被手改成非法（无修饰键/快捷键冲突）：回退默认，别带着坏状态跑
            return Self::default();
        }
        s
    }

    /// 写盘失败只丢持久化，内存状态已生效（下次启动回退旧值），与 ui-zoom.txt 同策略
    pub fn save(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir).and_then(|_| {
            serde_json::to_string_pretty(self)
                .map_err(|e| e.into())
                .and_then(|j| std::fs::write(file_path(dir), j))
        });
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, sc) in [
            (crate::i18n::pick("放大", "Zoom in"), &self.zoom_in),
            (crate::i18n::pick("缩小", "Zoom out"), &self.zoom_out),
        ] {
            if !(sc.ctrl || sc.shift || sc.alt) {
                return Err(crate::i18n::pick(
                    format!("{name}快捷键必须包含 Ctrl/Shift/Alt 中至少一个修饰键"),
                    format!("{name} shortcut must include at least one modifier (Ctrl/Shift/Alt)"),
                ));
            }
        }
        if self.zoom_in == self.zoom_out {
            return Err(crate::i18n::pick(
                "放大与缩小快捷键不能相同",
                "Zoom in and zoom out shortcuts must differ",
            )
            .into());
        }
        Ok(())
    }
}

/// 托管状态：内存值 + 持久化目录。set 先 clamp/校验，再落盘，最后替换内存值；
/// 校验失败时内存与磁盘都保持旧值。
pub struct SettingsState {
    dir: PathBuf,
    inner: Mutex<ShellSettings>,
}

impl SettingsState {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(ShellSettings::load(&dir)),
            dir,
        }
    }

    pub fn get(&self) -> ShellSettings {
        self.inner.lock().unwrap().clone()
    }

    pub fn set(&self, mut s: ShellSettings) -> Result<(), String> {
        s.zoom_step = s.zoom_step.clamp(STEP_MIN, STEP_MAX);
        s.validate()?;
        s.save(&self.dir);
        *self.inner.lock().unwrap() = s;
        Ok(())
    }
}

#[tauri::command]
pub fn get_shell_settings(state: tauri::State<SettingsState>) -> ShellSettings {
    state.get()
}

/// 保存设置；成功后重注入主窗口的缩放钩子（快捷键定义内嵌在脚本里，
/// 必须重注入才生效；钩子内部热替换监听器，不会叠加）
#[tauri::command]
pub fn set_shell_settings(
    app: tauri::AppHandle,
    state: tauri::State<SettingsState>,
    next: ShellSettings,
) -> Result<(), String> {
    state.set(next)?;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(crate::zoom::hook_js(&state.get()));
    }
    Ok(())
}

/// 试听任务完成提示音：内置预设走 toast 音频属性；自定义柔和音（Chime/Drop/
/// Mellow）弹静音 toast 并由壳播放内置 wav（文件缺失降级系统默认预设）。
#[tauri::command]
pub fn preview_completion_sound(
    app: tauri::AppHandle,
    platform: tauri::State<std::sync::Arc<dyn crate::platform::Platform>>,
    sound: CompletionSound,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let mut builder = app
        .notification()
        .builder()
        .title("DSHDesktop")
        .body(crate::i18n::pick("任务完成提示音试听", "Completion sound preview"));
    if let Some(rel) = sound.custom_wav() {
        match crate::resolve_custom_sound(&app, rel) {
            Some(p) => platform.play_sound_file(&p)?,
            None => builder = builder.sound("Default"),
        }
    } else if let Some(name) = sound.toast_sound_name() {
        builder = builder.sound(name);
    }
    builder.show().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_defaults_when_missing_or_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        // 文件不存在 → 全默认
        let s = ShellSettings::load(dir.path());
        assert!((s.zoom_step - 0.02).abs() < 1e-9);
        assert!(s.zoom_in.ctrl && s.zoom_in.shift && !s.zoom_in.alt);
        assert_eq!(s.zoom_in.code, "Equal");
        assert_eq!(s.zoom_in.key, "+");
        assert_eq!(s.zoom_out.code, "Minus");
        assert_eq!(s.zoom_out.key, "_");
        assert!(matches!(s.close_behavior, CloseBehavior::Background));
        // 损坏 JSON → 全默认
        std::fs::write(dir.path().join("settings.json"), "not json").unwrap();
        let s = ShellSettings::load(dir.path());
        assert!((s.zoom_step - 0.02).abs() < 1e-9);
    }

    #[test]
    fn load_partial_fields_filled_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{ "zoom_step": 0.05 }"#).unwrap();
        let s = ShellSettings::load(dir.path());
        assert!((s.zoom_step - 0.05).abs() < 1e-9);
        assert_eq!(s.zoom_in.code, "Equal"); // 未提供的字段回退默认
    }

    #[test]
    fn step_clamped_to_1_25_percent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{ "zoom_step": 0.5 }"#).unwrap();
        assert!((ShellSettings::load(dir.path()).zoom_step - 0.25).abs() < 1e-9);
        std::fs::write(dir.path().join("settings.json"), r#"{ "zoom_step": 0.001 }"#).unwrap();
        assert!((ShellSettings::load(dir.path()).zoom_step - 0.01).abs() < 1e-9);
    }

    #[test]
    fn validate_rejects_modifierless_and_conflicting_shortcuts() {
        let mut s = ShellSettings::default();
        s.zoom_in = Shortcut { ctrl: false, shift: false, alt: false, code: "KeyZ".into(), key: "z".into() };
        assert!(s.validate().is_err()); // 无修饰键

        let mut s = ShellSettings::default();
        s.zoom_out = s.zoom_in.clone();
        assert!(s.validate().is_err()); // in/out 冲突

        let mut s = ShellSettings::default();
        s.zoom_in = Shortcut { ctrl: true, shift: false, alt: true, code: "KeyZ".into(), key: "z".into() };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ShellSettings::default();
        s.zoom_step = 0.03;
        s.close_behavior = CloseBehavior::Quit;
        s.zoom_in = Shortcut { ctrl: true, shift: false, alt: true, code: "KeyQ".into(), key: "q".into() };
        s.save(dir.path());
        let s2 = ShellSettings::load(dir.path());
        assert!((s2.zoom_step - 0.03).abs() < 1e-9);
        assert!(matches!(s2.close_behavior, CloseBehavior::Quit));
        assert_eq!(s2.zoom_in.code, "KeyQ");
        assert!(s2.zoom_in.alt && !s2.zoom_in.shift);
    }

    #[test]
    fn state_set_validates_clamps_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let st = SettingsState::new(dir.path().to_path_buf());
        // 越界步进在 set 时 clamp，且同步落盘
        let mut s = st.get();
        s.zoom_step = 0.5;
        st.set(s).unwrap();
        assert!((st.get().zoom_step - 0.25).abs() < 1e-9);
        assert!((ShellSettings::load(dir.path()).zoom_step - 0.25).abs() < 1e-9);
        // 非法设置（无修饰键）被拒绝，内存值不变
        let mut bad = st.get();
        bad.zoom_in = Shortcut { ctrl: false, shift: false, alt: false, code: "KeyZ".into(), key: "z".into() };
        assert!(st.set(bad).is_err());
        assert!(st.get().zoom_in.ctrl);
    }

    #[test]
    fn completion_notify_defaults_on_and_sound_default() {
        let s = ShellSettings::default();
        assert!(s.notify_on_completion);
        assert_eq!(s.completion_sound, CompletionSound::Default);
    }

    #[test]
    fn old_settings_file_without_notify_fields_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        // 旧版配置文件：没有 notify_on_completion / completion_sound
        std::fs::write(dir.path().join("settings.json"), r#"{ "zoom_step": 0.05 }"#).unwrap();
        let s = ShellSettings::load(dir.path());
        assert!(s.notify_on_completion);
        assert_eq!(s.completion_sound, CompletionSound::Default);
    }

    #[test]
    fn completion_sound_roundtrip_and_serde_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ShellSettings::default();
        s.notify_on_completion = false;
        s.completion_sound = CompletionSound::Sms;
        s.save(dir.path());
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains(r#""completion_sound": "sms""#), "实际文件：{text}");
        let s2 = ShellSettings::load(dir.path());
        assert!(!s2.notify_on_completion);
        assert_eq!(s2.completion_sound, CompletionSound::Sms);
    }

    #[test]
    fn invalid_sound_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{ "completion_sound": "loud-noise" }"#,
        )
        .unwrap();
        let s = ShellSettings::load(dir.path());
        assert_eq!(s.completion_sound, CompletionSound::Default);
        assert!(s.notify_on_completion);
    }

    #[test]
    fn toast_sound_name_mapping() {
        assert_eq!(CompletionSound::Silent.toast_sound_name(), None);
        assert_eq!(CompletionSound::Default.toast_sound_name(), Some("Default"));
        assert_eq!(CompletionSound::Im.toast_sound_name(), Some("IM"));
        assert_eq!(CompletionSound::Mail.toast_sound_name(), Some("Mail"));
        assert_eq!(CompletionSound::Reminder.toast_sound_name(), Some("Reminder"));
        assert_eq!(CompletionSound::Sms.toast_sound_name(), Some("SMS"));
    }

    #[test]
    fn custom_soft_sounds_use_wav_not_toast_presets() {
        // 柔和系自定义音：toast 静音（None），由壳播放内置 wav
        for (s, wav) in [
            (CompletionSound::Chime, "sounds/chime.wav"),
            (CompletionSound::Drop, "sounds/drop.wav"),
            (CompletionSound::Mellow, "sounds/mellow.wav"),
        ] {
            assert_eq!(s.toast_sound_name(), None);
            assert_eq!(s.custom_wav(), Some(wav));
        }
        // 内置 toast 预设不对应自定义 wav
        assert_eq!(CompletionSound::Default.custom_wav(), None);
        assert_eq!(CompletionSound::Silent.custom_wav(), None);
    }

    #[test]
    fn custom_soft_sounds_serde_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ShellSettings::default();
        s.completion_sound = CompletionSound::Chime;
        s.save(dir.path());
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains(r#""completion_sound": "chime""#), "实际文件：{text}");
        assert_eq!(ShellSettings::load(dir.path()).completion_sound, CompletionSound::Chime);
    }

    #[test]
    fn shortcut_matches_by_code_or_key() {
        let sc = Shortcut { ctrl: true, shift: true, alt: false, code: "Equal".into(), key: "+".into() };
        // 真实键盘：code 命中
        assert!(sc.matches("Equal", "+", true, true, false));
        // 合成按键（code 为空）：key 命中
        assert!(sc.matches("", "+", true, true, false));
        // 修饰键不符 / 键不符 → 不命中
        assert!(!sc.matches("Equal", "+", true, false, false));
        assert!(!sc.matches("Minus", "_", true, true, false));
    }
}
