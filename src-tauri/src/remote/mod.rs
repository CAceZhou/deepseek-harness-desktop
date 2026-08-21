//! 远程访问：固定端口局域网暴露 + SSH 反向隧道 + Cloudflare Quick Tunnel +
//! 壳内嵌 token 鉴权反向代理。链路与安全模型见 docs/design.zh-CN.md；概览：
//!   a) 局域网：手机浏览器(同一局域网) ─HTTP→ 电脑 0.0.0.0:<固定端口>
//!        → remote::proxy.rs(token 门岗) → 127.0.0.1:dsh(完整 Web UI)
//!   b) 公网/异地（配置 SSH 隧道后）：浏览器 ─HTTP→ 自建公网服务器:<暴露端口>
//!        → SSH -R 反向隧道 → 电脑 127.0.0.1:<固定端口> → token 门岗 → dsh
//!   c) 公网（启用 Cloudflare Quick Tunnel 后）：浏览器 ─HTTPS→ Cloudflare
//!        → cloudflared(纯出站) → 127.0.0.1:<固定端口> → token 门岗 → dsh
//!
//! 三种对外方式并存：鉴权代理始终绑定 0.0.0.0:<固定端口>（局域网直连）；
//! SSH 与 Cloudflare 是独立的对外开关。对外访问链接按优先级取其一：
//! Cloudflare（公网可分享）> SSH（自建服务器）> 局域网 IP。
//!
//! RemoteManager 管生命周期：每次 start 重新生成 token，把鉴权代理绑定到设置里的
//! 固定端口（0.0.0.0 全接口）；再按启用的对外方式起 SSH 反向隧道和/或 cloudflared
//! quick tunnel，链接取当时生效的方式对应地址（Cloudflare → trycloudflare.com；
//! SSH → http://<服务器>:<暴露端口>；否则 → http://<本机局域网 IP>:<固定端口>）。
//! stop/退出应用即整体关停，链接立即失效。链接泄露时用 reset_link 原地轮换 token
//! 并掐断现有会话（地址与端口不变）。
pub mod proxy;
pub mod ssh_tunnel;
pub mod tunnel;

use crate::platform::Platform;
use crate::settings::SshTunnelSettings;
use proxy::{spawn_proxy, ProxyHandle};
use rand::Rng;
use serde::Serialize;
use ssh_tunnel::{SshEvent, SshState, SshTunnelProcess};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use tauri::State;
use tokio::sync::watch;
use tunnel::{TunnelEvent, TunnelProcess, TunnelState};

/// 远程访问运行配置的快照：由 settings.rs 的 set_shell_settings 保存设置后经
/// watch 通道推入，RemoteManager 每次 start 读当前值——改配置无需重启应用。
#[derive(Debug, Clone)]
pub struct RemoteSettings {
    /// 本地鉴权代理固定端口（0.0.0.0 绑定）
    pub port: u16,
    /// SSH 反向隧道配置（enabled 且 valid 时启用）
    pub ssh: SshTunnelSettings,
    /// Cloudflare Quick Tunnel 开关（cloudflared 出站隧道发布到公网）
    pub cloudflare: bool,
}

/// 上述配置的托管句柄（watch Sender 端）
pub struct RemoteConfig(pub watch::Sender<RemoteSettings>);

/// 每次开启远程访问重新生成的会话凭据：256-bit 随机，64 字符小写 hex
pub fn generate_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// 常数时间比较（等长时逐字节异或）；长度不等直接 false
pub(crate) fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 远程链接 = 访问地址 + 首次访问凭据
pub fn compose_link(url: &str, token: &str) -> String {
    format!("{url}/?token={token}")
}

/// SSH 隧道模式下链接用的服务器地址：剥掉用户可能填写的 http(s):// 前缀与尾部斜杠
pub fn ssh_url_host(server: &str) -> String {
    let t = server.trim();
    t.trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string()
}

/// SSH 模式下生成的访问地址（不含 token）：协议跟随 server 前缀（https:// → https，
/// 否则 http）；端口取 link_port（非 0 覆盖）否则 expose_port——供自建服务器用反向
/// 代理对外公布（对外端口 ≠ SSH -R 转发端口）时手动指定链接端口。
pub fn ssh_link_url(server: &str, expose_port: u16, link_port: u16) -> String {
    let t = server.trim();
    let (scheme, rest) = if let Some(r) = t.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = t.strip_prefix("http://") {
        ("http", r)
    } else {
        ("http", t)
    };
    let host = rest.trim_end_matches('/');
    let port = if link_port != 0 { link_port } else { expose_port };
    format!("{scheme}://{host}:{port}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Off,
    Starting,
    Up,
    Error,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Off => "off",
            Phase::Starting => "starting",
            Phase::Up => "up",
            Phase::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteStatus {
    /// "off" | "starting" | "up" | "error"
    pub phase: String,
    /// 访问地址（不含 token）：SSH 模式为 http://<服务器>:<暴露端口>，否则为
    /// http://<局域网 IP>:<固定端口>
    pub url: Option<String>,
    /// 完整访问链接（含 token），仅 Up 时存在；链接即凭据，勿分享
    pub link: Option<String>,
    pub error: Option<String>,
    /// 本地鉴权代理固定端口（诊断用）
    pub proxy_port: Option<u16>,
}

#[derive(Debug)]
pub enum RemoteEvent {
    Status(RemoteStatus),
    Log(String),
}

struct Inner {
    phase: Phase,
    token: Option<Arc<str>>,
    url: Option<String>,
    error: Option<String>,
    proxy_port: Option<u16>,
    proxy: Option<ProxyHandle>,
    ssh_tunnel: Option<SshTunnelProcess>,
    /// Cloudflare quick tunnel 监督进程（启用时存在）
    cf_tunnel: Option<TunnelProcess>,
    /// Cloudflare 是否为当前会话的对外链接主宰方式（= 本次 start 是否启用 cloudflare）。
    /// 为 true 时由 cloudflared 事件驱动 phase/link，SSH 事件只记日志不抢占状态；
    /// 为 false 时按原逻辑由 SSH（启用时）或局域网 IP 决定。
    cf_drives: bool,
}

impl Inner {
    fn dto(&self) -> RemoteStatus {
        let link = match (self.phase, &self.url, &self.token) {
            (Phase::Up, Some(url), Some(t)) => Some(compose_link(url, t)),
            _ => None,
        };
        RemoteStatus {
            phase: self.phase.as_str().into(),
            url: self.url.clone(),
            link,
            error: self.error.clone(),
            proxy_port: self.proxy_port,
        }
    }
}

/// on_event 与状态分离存放：回调在锁外触发，允许回调里回查 status()（托盘更新即如此）
struct Shared {
    inner: Mutex<Inner>,
    on_event: Box<dyn Fn(RemoteEvent) + Send + Sync>,
}

#[derive(Clone)]
pub struct RemoteManager {
    shared: Arc<Shared>,
    platform: Arc<dyn Platform>,
    /// 系统 OpenSSH 客户端路径（测试注入 node + prefix）
    ssh_exe: PathBuf,
    /// ssh 进程前置参数（测试注入 fixture 脚本路径；生产为空）
    ssh_prefix: Vec<String>,
    /// ssh 子进程的工作目录（生产 = 应用数据目录）
    ssh_work_dir: PathBuf,
    /// cloudflared 可执行路径（随 runtime 内嵌分发）
    tunnel_exe: PathBuf,
    /// cloudflared 进程前置参数（测试注入 fixture 脚本路径；生产为空）
    tunnel_prefix: Vec<String>,
    /// 运行配置（watch 通道：设置保存后立即生效，下次 start 用新值）
    config: watch::Receiver<RemoteSettings>,
    dsh_port: watch::Receiver<Option<u16>>,
}

impl RemoteManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        platform: Arc<dyn Platform>,
        ssh_exe: PathBuf,
        ssh_prefix: Vec<String>,
        ssh_work_dir: PathBuf,
        tunnel_exe: PathBuf,
        tunnel_prefix: Vec<String>,
        config: watch::Receiver<RemoteSettings>,
        dsh_port: watch::Receiver<Option<u16>>,
        on_event: Box<dyn Fn(RemoteEvent) + Send + Sync>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    phase: Phase::Off,
                    token: None,
                    url: None,
                    error: None,
                    proxy_port: None,
                    proxy: None,
                    ssh_tunnel: None,
                    cf_tunnel: None,
                    cf_drives: false,
                }),
                on_event,
            }),
            platform,
            ssh_exe,
            ssh_prefix,
            ssh_work_dir,
            tunnel_exe,
            tunnel_prefix,
            config,
            dsh_port,
        }
    }

    pub fn status(&self) -> RemoteStatus {
        self.shared.inner.lock().unwrap().dto()
    }

    pub async fn start(&self) -> RemoteStatus {
        {
            let mut g = self.shared.inner.lock().unwrap();
            if matches!(g.phase, Phase::Starting | Phase::Up) {
                return g.dto();
            }
            // 先占位再异步起服务：并发 start 在锁内看到 Starting 直接返回，
            // 否则两个 start 会各起一套代理/隧道，先起的那套句柄被覆盖丢失
            g.phase = Phase::Starting;
            g.error = None;
        }
        let cfg = self.config.borrow().clone();
        let port = cfg.port;
        let bind: std::net::SocketAddr = match format!("0.0.0.0:{port}").parse() {
            Ok(a) => a,
            Err(_) => {
                return self
                    .transition_error(format!("远程访问端口 {port} 无效，请在“其它设置”中更换"));
            }
        };
        let token: Arc<str> = generate_token().into();
        let proxy = match spawn_proxy(token.clone(), self.dsh_port.clone(), bind).await {
            Ok(p) => p,
            Err(e) => {
                let msg = if e.kind() == io::ErrorKind::AddrInUse {
                    format!("远程访问端口 {port} 已被占用，请在“其它设置”中更换端口")
                } else {
                    format!("远程代理启动失败：{e}")
                };
                return self.transition_error(msg);
            }
        };
        let proxy_port = proxy.port;

        // 对外链接主宰方式：Cloudflare > SSH > 局域网。cloudflared 启用时需要 exe 存在。
        let cf_drives = cfg.cloudflare;
        if cfg.cloudflare && !self.tunnel_exe.is_file() {
            proxy.shutdown().await;
            return self.transition_error(format!(
                "cloudflared 缺失（{}），请重新安装或重新运行 fetch-runtime",
                self.tunnel_exe.display()
            ));
        }
        if cfg.ssh.enabled && !cfg.ssh.valid() {
            proxy.shutdown().await;
            return self.transition_error(
                "SSH 隧道配置不完整：服务器地址、用户名、私钥路径必填，端口需在 1-65535"
                    .into(),
            );
        }

        // SSH 反向隧道（enabled 时；无论是否 Cloudflare 主宰都起，两者可并存）
        let ssh_tunnel = if cfg.ssh.enabled {
            let host = ssh_url_host(&cfg.ssh.server);
            let weak: Weak<Shared> = Arc::downgrade(&self.shared);
            Some(SshTunnelProcess::spawn_supervised(
                self.platform.clone(),
                self.ssh_exe.clone(),
                self.ssh_prefix.clone(),
                cfg.ssh.ssh_port,
                &cfg.ssh.user,
                &host,
                std::path::Path::new(&cfg.ssh.key_path),
                cfg.ssh.expose_port,
                proxy_port,
                self.ssh_work_dir.clone(),
                move |ev| {
                    let Some(shared) = weak.upgrade() else { return };
                    handle_ssh_event(&shared, ev);
                },
            ))
        } else {
            None
        };

        // Cloudflare quick tunnel（enabled 时）：cloudflared 出站隧道指向本地鉴权代理
        let cf_tunnel = if cfg.cloudflare {
            let weak: Weak<Shared> = Arc::downgrade(&self.shared);
            let target = format!("http://127.0.0.1:{proxy_port}");
            Some(TunnelProcess::spawn_supervised(
                self.platform.clone(),
                self.tunnel_exe.clone(),
                self.tunnel_prefix.clone(),
                target,
                self.ssh_work_dir.clone(),
                move |ev| {
                    let Some(shared) = weak.upgrade() else { return };
                    handle_cf_event(&shared, ev);
                },
            ))
        } else {
            None
        };

        // 初始 url/phase：Cloudflare 或 SSH 主宰时保持 Starting（等隧道就绪才 Up）；
        // 两者都关时局域网直连即 up。链接地址由主宰方式决定：
        //   Cloudflare → 隧道 URL（handler 上报）；SSH → 服务器地址；否则局域网 IP。
        let (url, starting) = if cf_drives {
            (None, true)
        } else if ssh_tunnel.is_some() {
            (
                Some(ssh_link_url(
                    &cfg.ssh.server,
                    cfg.ssh.expose_port,
                    cfg.ssh.link_port,
                )),
                true,
            )
        } else {
            (Some(format!("http://{}:{proxy_port}", lan_ipv4())), false)
        };

        let st = {
            let mut g = self.shared.inner.lock().unwrap();
            // SSH/Cloudflare 模式：等隧道就绪后才 Up（避免链接 URL 对外但隧道未建立）
            g.phase = if starting { Phase::Starting } else { Phase::Up };
            g.token = Some(token);
            g.url = url;
            g.error = None;
            g.proxy_port = Some(proxy_port);
            g.proxy = Some(proxy);
            g.ssh_tunnel = ssh_tunnel;
            g.cf_tunnel = cf_tunnel;
            g.cf_drives = cf_drives;
            g.dto()
        };
        (self.shared.on_event)(RemoteEvent::Log(format!(
            "[dshdesktop] remote access: proxy 0.0.0.0:{proxy_port} (cf_drives={cf_drives}) \
             (token 仅存于链接)"
        )));
        (self.shared.on_event)(RemoteEvent::Status(st.clone()));
        st
    }

    pub async fn stop(&self) -> RemoteStatus {
        let (proxy, ssh_tunnel, cf_tunnel) = {
            let mut g = self.shared.inner.lock().unwrap();
            g.phase = Phase::Off;
            g.token = None;
            g.url = None;
            g.error = None;
            g.proxy_port = None;
            g.cf_drives = false;
            (g.proxy.take(), g.ssh_tunnel.take(), g.cf_tunnel.take())
        };
        if let Some(t) = ssh_tunnel {
            t.stop().await;
        }
        if let Some(t) = cf_tunnel {
            t.stop().await;
        }
        if let Some(p) = proxy {
            p.shutdown().await;
        }
        let st = self.status();
        (self.shared.on_event)(RemoteEvent::Status(st.clone()));
        st
    }

    /// 重置访问链接：原地轮换 token 并掐断所有已建立会话（代理门岗逐请求读
    /// 最新 token，旧链接/旧 cookie 立即失效；WS 桥接被 drain 掐断）。
    /// 地址与端口保持不变，无需重新建立。链接泄露后的吊销手段。
    pub fn reset_link(&self) -> Result<RemoteStatus, String> {
        let st = {
            let mut g = self.shared.inner.lock().unwrap();
            if !matches!(g.phase, Phase::Starting | Phase::Up) {
                return Err("远程访问未开启".into());
            }
            let Some(proxy) = &g.proxy else {
                return Err("远程访问代理未就绪，请稍后重试".into());
            };
            let token: Arc<str> = generate_token().into();
            proxy.reset_token(token.clone());
            g.token = Some(token);
            g.dto()
        };
        (self.shared.on_event)(RemoteEvent::Status(st.clone()));
        Ok(st)
    }

    fn transition_error(&self, msg: String) -> RemoteStatus {
        let st = {
            let mut g = self.shared.inner.lock().unwrap();
            g.phase = Phase::Error;
            g.error = Some(msg);
            g.dto()
        };
        (self.shared.on_event)(RemoteEvent::Status(st.clone()));
        st
    }
}

fn handle_ssh_event(shared: &Shared, ev: SshEvent) {
    match ev {
        SshEvent::Log(l) => (shared.on_event)(RemoteEvent::Log(l)),
        SshEvent::StateChanged(ss) => {
            let st = {
                let mut g = shared.inner.lock().unwrap();
                // stop() 后迟到的隧道事件不得把状态复活（隧道停杀与事件上报有窗口期）
                if g.phase == Phase::Off {
                    return;
                }
                // Cloudflare 为当前对外的链接主宰方式时，SSH 隧道只在后台跑，
                // 其状态不抢占 phase/link（只记日志）；避免两台隧道互相覆盖链接。
                if g.cf_drives {
                    return;
                }
                match ss {
                    SshState::Up => {
                        // 隧道就绪：链接地址此前已按服务器地址生成，这里只落 Up
                        g.phase = Phase::Up;
                        g.error = None;
                    }
                    SshState::Starting => {
                        g.phase = Phase::Starting;
                    }
                    SshState::Failed(msg) => {
                        g.phase = Phase::Error;
                        g.error = Some(format!("SSH 隧道失败：{msg}"));
                    }
                    SshState::Stopped => return,
                }
                g.dto()
            };
            (shared.on_event)(RemoteEvent::Status(st));
        }
    }
}

/// cloudflared quick tunnel 事件：启用时它主宰对外链接，Up 时把 trycloudflare URL
/// 落进状态生成带 token 的链接；Starting 保持 Starting；Failed 进入 Error。
fn handle_cf_event(shared: &Shared, ev: TunnelEvent) {
    match ev {
        TunnelEvent::Log(l) => (shared.on_event)(RemoteEvent::Log(l)),
        TunnelEvent::StateChanged(ts) => {
            let st = {
                let mut g = shared.inner.lock().unwrap();
                // stop() 后迟到的隧道事件不得把状态复活（隧道停杀与事件上报有窗口期）
                if g.phase == Phase::Off {
                    return;
                }
                // 仅当本次会话 Cloudflare 是主宰方式时才由它驱动 phase/link
                if !g.cf_drives {
                    return;
                }
                match ts {
                    TunnelState::Up { url } => {
                        g.phase = Phase::Up;
                        g.url = Some(url);
                        g.error = None;
                    }
                    // 隧道重连：token 与代理保留，链接随新 URL 重新生成
                    TunnelState::Starting => {
                        g.phase = Phase::Starting;
                        g.url = None;
                    }
                    TunnelState::Failed(msg) => {
                        g.phase = Phase::Error;
                        g.url = None;
                        g.error = Some(format!("cloudflared 隧道失败：{msg}"));
                    }
                    // 仅由 stop() 触发，那里已落 Off 并上报
                    TunnelState::Stopped => return,
                }
                g.dto()
            };
            (shared.on_event)(RemoteEvent::Status(st));
        }
    }
}

/// 本机局域网 IPv4：UDP connect 不发包，内核按默认路由选接口后 local_addr
/// 即局域网地址（VPN 场景取到的是 VPN 网卡地址，同样可达）。无默认路由
/// （完全离线）时回退 127.0.0.1——此时链接本就不通，状态仍可展示。
fn lan_ipv4() -> String {
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                if let std::net::IpAddr::V4(v4) = addr.ip() {
                    return v4.to_string();
                }
            }
        }
    }
    "127.0.0.1".into()
}

#[tauri::command]
pub async fn start_remote(state: State<'_, RemoteManager>) -> Result<RemoteStatus, String> {
    Ok(state.start().await)
}

#[tauri::command]
pub async fn stop_remote(state: State<'_, RemoteManager>) -> Result<RemoteStatus, String> {
    Ok(state.stop().await)
}

#[tauri::command]
pub fn get_remote_status(state: State<'_, RemoteManager>) -> RemoteStatus {
    state.status()
}

#[tauri::command]
pub fn copy_remote_link(state: State<'_, RemoteManager>) -> Result<(), String> {
    copy_link_to_clipboard(&state)
}

/// 重置访问链接（token 轮换 + 掐断现有会话），地址与端口不变
#[tauri::command]
pub fn reset_remote_link(state: State<'_, RemoteManager>) -> Result<RemoteStatus, String> {
    state.reset_link()
}

/// 托盘菜单与 invoke 命令共用的复制逻辑
pub fn copy_link_to_clipboard(mgr: &RemoteManager) -> Result<(), String> {
    let link = mgr.status().link.ok_or("远程访问尚未就绪")?;
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用：{e}"))?;
    cb.set_text(link).map_err(|e| format!("复制失败：{e}"))
}

/// 当前链接的二维码（SVG 字符串）；仅 Up 态可用
#[tauri::command]
pub fn get_remote_qr(state: State<'_, RemoteManager>) -> Result<String, String> {
    let link = state.status().link.ok_or("远程访问尚未就绪")?;
    let code = qrcode::QrCode::new(link.as_bytes()).map_err(|e| format!("二维码生成失败：{e}"))?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .build())
}

/// 写 events.log 前脱敏：链接即凭据，日志里不能出现 token
/// （任何外部命令/请求日志若带 ?token= 查询串都会被替换）
pub(crate) fn redact_token(s: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r#"(?i)token=[^&\s"']+"#).unwrap());
    re.replace_all(s, "token=<redacted>").into_owned()
}

#[cfg(test)]
mod tests {
    use super::{compose_link, lan_ipv4, redact_token, ssh_link_url, ssh_url_host};

    #[test]
    fn redact_token_strips_credential() {
        assert_eq!(
            redact_token("dest=http://192.168.1.5:7788/?token=abc123xyz&type=http"),
            "dest=http://192.168.1.5:7788/?token=<redacted>&type=http"
        );
        assert_eq!(
            redact_token("link http://example.com:8080/?token=deadbeef"),
            "link http://example.com:8080/?token=<redacted>"
        );
        assert_eq!(redact_token("no token here"), "no token here");
    }

    #[test]
    fn compose_link_appends_token() {
        assert_eq!(
            compose_link("http://192.168.1.5:7788", "abc123"),
            "http://192.168.1.5:7788/?token=abc123"
        );
    }

    #[test]
    fn ssh_url_host_strips_protocol_and_slash() {
        assert_eq!(ssh_url_host("1.2.3.4"), "1.2.3.4");
        assert_eq!(ssh_url_host("http://vps.example.com"), "vps.example.com");
        assert_eq!(ssh_url_host("https://vps.example.com/"), "vps.example.com");
        assert_eq!(ssh_url_host("  my-host.com  "), "my-host.com");
    }

    #[test]
    fn ssh_link_url_follows_protocol_and_port_override() {
        // 默认：http + 暴露端口
        assert_eq!(
            ssh_link_url("vps.example.com", 8080, 0),
            "http://vps.example.com:8080"
        );
        // link_port 覆盖（反向代理对外端口 ≠ 转发端口）
        assert_eq!(
            ssh_link_url("vps.example.com", 8080, 8443),
            "http://vps.example.com:8443"
        );
        // 服务器地址带 https:// → 链接走 https；443 等默认端口也照写
        assert_eq!(
            ssh_link_url("https://vps.example.com/", 8080, 0),
            "https://vps.example.com:8080"
        );
        assert_eq!(
            ssh_link_url("https://vps.example.com", 8080, 443),
            "https://vps.example.com:443"
        );
    }

    #[test]
    fn lan_ipv4_returns_parseable_address() {
        // 任何环境下都必须返回可解析的 IPv4（离线时回退 127.0.0.1）
        let ip = lan_ipv4();
        let parsed: std::net::Ipv4Addr = ip.parse().expect("应为合法 IPv4");
        assert!(!parsed.is_unspecified());
    }
}
