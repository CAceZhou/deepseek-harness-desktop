use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

pub const STEP_MIN: f64 = 0.01;
pub const STEP_MAX: f64 = 0.25;
/// 远程访问固定端口默认值（0.0.0.0 全接口监听，局域网内访问）
pub const REMOTE_PORT_DEFAULT: u16 = 7788;
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

/// 通知时机：Background = 仅当应用无聚焦窗口（后台）时提醒；Always = 前台也提醒
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotifyTiming {
    #[default]
    Background,
    Always,
}

/// 一类通知的规则：开关 + 时机
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NotifyRule {
    pub enabled: bool,
    pub timing: NotifyTiming,
}

impl Default for NotifyRule {
    fn default() -> Self {
        Self { enabled: true, timing: NotifyTiming::Background }
    }
}

impl NotifyRule {
    /// foreground = 本应用任一窗口处于聚焦态
    pub fn allows(&self, foreground: bool) -> bool {
        self.enabled && (self.timing == NotifyTiming::Always || !foreground)
    }
}

/// 三类通知的独立规则：待批准 / 待回答 / 回答完毕（任务完成）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct NotifySettings {
    pub approval: NotifyRule,
    pub question: NotifyRule,
    pub turn_done: NotifyRule,
}

/// SSH 反向隧道配置：把本地鉴权代理的固定端口经 SSH -R 转发到自建公网服务器，
/// 公网/异地凭 `http://<server>:<expose_port>` 访问。私钥鉴权（OpenSSH 格式）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SshTunnelSettings {
    /// 开启 SSH 隧道（enabled 时 server/user/key_path 必填、端口必须合法）
    pub enabled: bool,
    /// 目标服务器地址（IP 或域名，可带 http(s):// 前缀）；SSH 模式下生成链接的
    /// 地址与此一致，协议也跟随前缀（https:// → 生成 https 链接）
    pub server: String,
    /// 服务器 SSH 端口（默认 22）
    pub ssh_port: u16,
    /// SSH 登录用户名
    pub user: String,
    /// 鉴权私钥文件路径（OpenSSH 格式，无口令或经 ssh-agent）
    pub key_path: String,
    /// 服务器上暴露的端口（反向转发目标端口，SSH -R 实际绑定的端口）
    pub expose_port: u16,
    /// 生成访问链接时覆盖的端口号：0 = 跟随 expose_port；非 0 = 链接用它。
    /// 供自建服务器上用反向代理（Nginx/Caddy 等）对外公布、对外端口 ≠ 转发端口的场景。
    pub link_port: u16,
}

impl Default for SshTunnelSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            server: String::new(),
            ssh_port: 22,
            user: String::new(),
            key_path: String::new(),
            expose_port: 0,
            link_port: 0,
        }
    }
}

impl SshTunnelSettings {
    /// enabled 时配置是否可用的校验：server/user/key 非空，SSH 端口与暴露端口在 1..=65535。
    /// link_port 是 u16：0 = 跟随暴露端口，非 0 必在 1..=65535，无需额外校验。
    pub fn valid(&self) -> bool {
        if !self.enabled {
            return true;
        }
        !self.server.trim().is_empty()
            && !self.user.trim().is_empty()
            && !self.key_path.trim().is_empty()
            && (1..=65535).contains(&self.ssh_port)
            && (1..=65535).contains(&self.expose_port)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellSettings {
    pub zoom_step: f64,
    pub zoom_in: Shortcut,
    pub zoom_out: Shortcut,
    pub close_behavior: CloseBehavior,
    pub notify: NotifySettings,
    pub completion_sound: CompletionSound,
    /// 启动时自动检查更新（默认关）：开启后每次启动后台查 GitHub releases，有新版弹 toast
    pub check_update_on_launch: bool,
    /// 远程访问固定端口（默认 7788）：0 视作默认（load/set 时归一化）
    pub remote_port: u16,
    /// SSH 反向隧道配置（内网穿透到自建公网服务器）
    pub ssh_tunnel: SshTunnelSettings,
    /// Cloudflare Quick Tunnel（上游原本的远程方式）：本机 cloudflared 出站
    /// 隧道把本地鉴权代理发布到公网 trycloudflare 域名，无需公网服务器端口映射。
    pub cloudflare_tunnel: bool,
    /// 允许明文 HTTP 访问（默认关）：远程访问默认只走 HTTPS（Cloudflare 隧道、
    /// 或自建服务器 TLS 反代）；开启后局域网直连与 SSH 非 TLS 暴露端口可用
    /// http:// 访问，明文传输不安全，仅建议可信网络使用。
    pub allow_http: bool,
    /// 旧版字段（≤0.1.7）：读取时迁移进 notify.turn_done.enabled，保存时不再写出
    #[serde(skip_serializing)]
    notify_on_completion: Option<bool>,
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
            notify: NotifySettings::default(),
            completion_sound: CompletionSound::Default,
            check_update_on_launch: false,
            remote_port: REMOTE_PORT_DEFAULT,
            ssh_tunnel: SshTunnelSettings::default(),
            cloudflare_tunnel: false,
            allow_http: false,
            notify_on_completion: None,
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
        // 旧版 notify_on_completion 布尔 → notify.turn_done.enabled（其余类型默认开）
        if let Some(b) = s.notify_on_completion.take() {
            s.notify.turn_done.enabled = b;
        }
        s.zoom_step = s.zoom_step.clamp(STEP_MIN, STEP_MAX);
        if s.remote_port == 0 {
            s.remote_port = REMOTE_PORT_DEFAULT;
        }
        if s.ssh_tunnel.ssh_port == 0 {
            s.ssh_tunnel.ssh_port = 22;
        }
        // SSH 配置不合法（如被手改成半截）：整体回退默认（关），不拖垮其它设置
        if !s.ssh_tunnel.valid() {
            s.ssh_tunnel = SshTunnelSettings::default();
        }
        if s.validate().is_err() {
            // 配置文件被手改成非法（无修饰键/快捷键冲突）：回退默认，别带着坏状态跑
            return Self::default();
        }
        s
    }

    /// 写盘失败显式上报：旧版静默吞掉（仅丢持久化），但若写盘被环境阻断
    /// （杀软目录保护等），用户看到的是"保存成功/无关报错"而重启后设置回退，
    /// 无法感知真正原因。失败时内存值也不替换，界面状态与磁盘保持一致。
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let j = serde_json::to_string_pretty(self).map_err(std::io::Error::from)?;
        std::fs::write(file_path(dir), j)
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
        // SSH 隧道开启时必填项检查（端口越界、字段为空都拒收）
        if !self.ssh_tunnel.valid() {
            return Err(crate::i18n::pick(
                "SSH 隧道配置不完整：服务器地址、用户名、私钥路径必填，SSH 端口与暴露端口需在 1-65535",
                "SSH tunnel config incomplete: server, user and key path are required, and SSH/expose ports must be 1-65535",
            )
            .into());
        }
        Ok(())
    }
}

/// 托管状态：内存值 + 持久化目录。set 先 clamp/校验，再落盘，最后替换内存值；
/// 校验或落盘失败时内存与磁盘都保持旧值。
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
        if s.remote_port == 0 {
            s.remote_port = REMOTE_PORT_DEFAULT;
        }
        if s.ssh_tunnel.ssh_port == 0 {
            s.ssh_tunnel.ssh_port = 22;
        }
        s.validate()?;
        s.save(&self.dir).map_err(|e| {
            crate::i18n::pick(
                format!("设置写入失败: {e}"),
                format!("Failed to write settings file: {e}"),
            )
        })?;
        *self.inner.lock().unwrap() = s;
        Ok(())
    }
}

#[tauri::command]
pub fn get_shell_settings(state: tauri::State<SettingsState>) -> ShellSettings {
    state.get()
}

/// 保存设置；成功后重注入主窗口的缩放钩子（快捷键定义内嵌在脚本里，
/// 必须重注入才生效；钩子内部热替换监听器，不会叠加）；
/// 远程访问运行配置（固定端口 + SSH 隧道）同步进 RemoteConfig 通道——
/// RemoteManager 下次 start 即用新配置（无需重启应用）
#[tauri::command]
pub fn set_shell_settings(
    app: tauri::AppHandle,
    state: tauri::State<SettingsState>,
    next: ShellSettings,
) -> Result<(), String> {
    state.set(next)?;
    if let Some(cfg) = app.try_state::<crate::remote::RemoteConfig>() {
        let s = state.get();
        let _ = cfg.0.send(crate::remote::RemoteSettings {
            port: s.remote_port,
            ssh: s.ssh_tunnel.clone(),
            cloudflare: s.cloudflare_tunnel,
            allow_http: s.allow_http,
        });
    }
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
        assert_eq!(s.remote_port, REMOTE_PORT_DEFAULT, "未提供的端口回退默认 7788");
    }

    #[test]
    fn remote_port_zero_normalizes_to_default() {
        let dir = tempfile::tempdir().unwrap();
        // 0 或缺失 → 默认 7788；合法值原样保留并往返
        std::fs::write(dir.path().join("settings.json"), r#"{ "remote_port": 0 }"#).unwrap();
        assert_eq!(ShellSettings::load(dir.path()).remote_port, REMOTE_PORT_DEFAULT);
        let mut s = ShellSettings::default();
        s.remote_port = 8000;
        s.save(dir.path()).unwrap();
        assert_eq!(ShellSettings::load(dir.path()).remote_port, 8000);
        // set 同样归一化 0 → 默认
        let st = SettingsState::new(dir.path().to_path_buf());
        let mut bad = st.get();
        bad.remote_port = 0;
        st.set(bad).unwrap();
        assert_eq!(st.get().remote_port, REMOTE_PORT_DEFAULT);
    }

    #[test]
    fn ssh_tunnel_defaults_off_and_validity_matrix() {
        let d = SshTunnelSettings::default();
        assert!(!d.enabled);
        assert!(d.valid(), "未开启时不需要填任何字段");
        assert_eq!(d.ssh_port, 22);

        // enabled 时必填项与端口范围
        let mut s = SshTunnelSettings {
            enabled: true,
            server: "1.2.3.4".into(),
            ssh_port: 22,
            user: "root".into(),
            key_path: r"C:\keys\id_ed25519".into(),
            expose_port: 8080,
            link_port: 0,
        };
        assert!(s.valid());
        s.server.clear();
        assert!(!s.valid(), "缺服务器地址");
        s.server = "1.2.3.4".into();
        s.user.clear();
        assert!(!s.valid(), "缺用户名");
        s.user = "root".into();
        s.key_path.clear();
        assert!(!s.valid(), "缺私钥路径");
        s.key_path = r"C:\keys\id_ed25519".into();
        s.expose_port = 0;
        assert!(!s.valid(), "暴露端口 0 非法");
        s.expose_port = 8080;
        assert!(s.valid(), "link_port=0 跟随暴露端口，配置仍合法");
        s.link_port = 8443;
        assert!(s.valid(), "link_port 覆盖值无需额外校验");
    }

    #[test]
    fn ssh_tunnel_roundtrip_and_invalid_load_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ShellSettings::default();
        s.ssh_tunnel = SshTunnelSettings {
            enabled: true,
            server: "vps.example.com".into(),
            ssh_port: 2222,
            user: "deploy".into(),
            key_path: r"C:\Users\me\.ssh\id_ed25519".into(),
            expose_port: 8080,
            link_port: 8443,
        };
        s.save(dir.path()).unwrap();
        let s2 = ShellSettings::load(dir.path());
        assert_eq!(s2.ssh_tunnel, s.ssh_tunnel, "SSH 配置应完整往返");

        // 半截配置（缺用户名）→ load 整体回退默认（关），不拖垮其它设置
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{ "zoom_step": 0.05, "ssh_tunnel": { "enabled": true, "server": "1.2.3.4", "ssh_port": 22, "key_path": "k", "expose_port": 8080 } }"#,
        )
        .unwrap();
        let s3 = ShellSettings::load(dir.path());
        assert!(s3.ssh_tunnel == SshTunnelSettings::default(), "非法 SSH 配置应回退默认");
        assert!((s3.zoom_step - 0.05).abs() < 1e-9, "其它设置不受影响");

        // set 校验拒绝非法 SSH 配置（enabled 但缺字段）
        let st = SettingsState::new(dir.path().to_path_buf());
        let mut bad = st.get();
        bad.ssh_tunnel.enabled = true;
        bad.ssh_tunnel.user.clear();
        assert!(st.set(bad).is_err());
        assert!(!st.get().ssh_tunnel.enabled, "非法配置不得替换内存值");
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
        s.save(dir.path()).unwrap();
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
        assert!(s.notify.turn_done.enabled);
        assert_eq!(s.completion_sound, CompletionSound::Default);
    }

    #[test]
    fn notify_rule_allows_matrix() {
        let on_bg = NotifyRule { enabled: true, timing: NotifyTiming::Background };
        let on_always = NotifyRule { enabled: true, timing: NotifyTiming::Always };
        let off = NotifyRule { enabled: false, timing: NotifyTiming::Always };
        assert!(on_bg.allows(false) && !on_bg.allows(true)); // 仅后台：前台不弹
        assert!(on_always.allows(false) && on_always.allows(true)); // 总是
        assert!(!off.allows(false) && !off.allows(true));
    }

    #[test]
    fn notify_settings_defaults() {
        let s = ShellSettings::default();
        for rule in [s.notify.approval, s.notify.question, s.notify.turn_done] {
            assert!(rule.enabled);
            assert_eq!(rule.timing, NotifyTiming::Background);
        }
    }

    #[test]
    fn legacy_notify_on_completion_migrates_to_turn_done() {
        let dir = tempfile::tempdir().unwrap();
        // 旧版文件（≤0.1.7）：只有 notify_on_completion 布尔，没有 notify 对象
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{ "notify_on_completion": false, "completion_sound": "sms" }"#,
        )
        .unwrap();
        let s = ShellSettings::load(dir.path());
        assert!(!s.notify.turn_done.enabled, "旧开关值应迁移到 turn_done");
        assert!(s.notify.approval.enabled, "其余类型取默认开");
        assert_eq!(s.completion_sound, CompletionSound::Sms);
        // 保存后旧字段消失、新结构落盘
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!text.contains("notify_on_completion"), "实际文件：{text}");
        assert!(text.contains(r#""turn_done""#), "实际文件：{text}");
    }

    #[test]
    fn notify_rule_serde_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ShellSettings::default();
        s.notify.turn_done.timing = NotifyTiming::Always;
        s.notify.approval.enabled = false;
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains(r#""timing": "always""#), "实际文件：{text}");
        assert!(text.contains(r#""enabled": false"#), "实际文件：{text}");
        let s2 = ShellSettings::load(dir.path());
        assert_eq!(s2.notify.turn_done.timing, NotifyTiming::Always);
        assert!(!s2.notify.approval.enabled);
    }

    #[test]
    fn old_settings_file_without_notify_fields_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        // 旧版配置文件：没有 notify / completion_sound
        std::fs::write(dir.path().join("settings.json"), r#"{ "zoom_step": 0.05 }"#).unwrap();
        let s = ShellSettings::load(dir.path());
        assert!(s.notify.turn_done.enabled);
        assert_eq!(s.notify.turn_done.timing, NotifyTiming::Background);
        assert_eq!(s.completion_sound, CompletionSound::Default);
    }

    #[test]
    fn check_update_on_launch_defaults_off_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        // 旧版文件没有该字段 → 默认关
        std::fs::write(dir.path().join("settings.json"), r#"{ "zoom_step": 0.05 }"#).unwrap();
        assert!(!ShellSettings::load(dir.path()).check_update_on_launch);
        // 开启后保存/读取往返一致
        let mut s = ShellSettings::default();
        s.check_update_on_launch = true;
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains(r#""check_update_on_launch": true"#), "实际文件：{text}");
        assert!(ShellSettings::load(dir.path()).check_update_on_launch);
    }

    #[test]
    fn allow_http_defaults_off_and_roundtrips() {
        // 明文 HTTP 默认必须关闭（远程访问默认只走 HTTPS）
        assert!(!ShellSettings::default().allow_http);
        let dir = tempfile::tempdir().unwrap();
        // 旧版文件没有该字段 → 默认关
        std::fs::write(dir.path().join("settings.json"), r#"{ "zoom_step": 0.05 }"#).unwrap();
        assert!(!ShellSettings::load(dir.path()).allow_http);
        // 开启后保存/读取往返一致
        let mut s = ShellSettings::default();
        s.allow_http = true;
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains(r#""allow_http": true"#), "实际文件：{text}");
        assert!(ShellSettings::load(dir.path()).allow_http);
    }

    #[test]
    fn completion_sound_roundtrip_and_serde_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ShellSettings::default();
        s.notify.turn_done.enabled = false;
        s.completion_sound = CompletionSound::Sms;
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains(r#""completion_sound": "sms""#), "实际文件：{text}");
        let s2 = ShellSettings::load(dir.path());
        assert!(!s2.notify.turn_done.enabled);
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
        assert!(s.notify.turn_done.enabled);
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
    fn bundled_sound_layout_matches_probe_path() {
        // resolve_custom_sound 在 resource_dir()/exe 旁的 sounds/ 下找 wav（custom_wav
        // 的相对路径），所以 bundle.resources 必须把 resources/sounds 映射为安装根的
        // sounds/。列表形式会原样保留目录层级，安装后落在 resources/sounds/ ——
        // 探测不到，自定义音静默降级系统默认（≤0.1.16 实踩）。
        let conf: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(conf["bundle"]["resources"]["resources/sounds"], "sounds");
        for s in [CompletionSound::Chime, CompletionSound::Drop, CompletionSound::Mellow] {
            assert!(s.custom_wav().unwrap().starts_with("sounds/"));
        }
    }

    #[test]
    fn custom_soft_sounds_serde_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ShellSettings::default();
        s.completion_sound = CompletionSound::Chime;
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.contains(r#""completion_sound": "chime""#), "实际文件：{text}");
        assert_eq!(ShellSettings::load(dir.path()).completion_sound, CompletionSound::Chime);
    }

    #[test]
    fn save_reports_io_error_instead_of_silently_dropping() {
        let dir = tempfile::tempdir().unwrap();
        // 落盘目录被同名文件占用 → create_dir_all 必失败；旧版静默吞掉这个错误
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, "x").unwrap();
        assert!(ShellSettings::default().save(&blocker).is_err());
        // set 同样透出写盘错误，且内存值不被替换
        let st = SettingsState::new(blocker);
        let mut s = st.get();
        s.zoom_step = 0.05;
        assert!(st.set(s).is_err());
        assert!((st.get().zoom_step - 0.02).abs() < 1e-9);
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
