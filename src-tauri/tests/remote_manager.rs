use dshdesktop_lib::platform::Platform;
use dshdesktop_lib::port::free_port;
use dshdesktop_lib::remote::{compose_link, RemoteEvent, RemoteManager, RemoteSettings, RemoteStatus};
use dshdesktop_lib::settings::SshTunnelSettings;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

struct TestPlatform;

impl Platform for TestPlatform {
    fn node_exe_name(&self) -> &'static str {
        "node.exe"
    }
    fn cloudflared_exe_name(&self) -> &'static str {
        "cloudflared.exe"
    }
    fn runtime_base_dir(&self) -> PathBuf {
        PathBuf::from(".")
    }
    fn resource_runtime_dir(&self, _: &Path) -> PathBuf {
        PathBuf::from(".")
    }
    fn runtime_triplet(&self) -> &'static str {
        "windows-x64"
    }
    fn kill_process_tree(&self, pid: u32) {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
    fn system_dark_mode(&self) -> bool {
        false
    }
    fn system_prefers_chinese(&self) -> bool {
        false
    }
    fn play_sound_file(&self, _path: &Path) -> Result<(), String> {
        Ok(())
    }
}

/// 直连（绕开系统代理）GET，供 fixture 就绪探测用
async fn get_direct(url: &str) -> reqwest::Result<reqwest::Response> {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
}

fn system_node() -> PathBuf {
    let out = Command::new("where").arg("node").output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    PathBuf::from(
        stdout
            .lines()
            .next()
            .expect("node not found on PATH")
            .trim(),
    )
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn spawn_fixture_dsh(port: u16, work: &Path) -> std::process::Child {
    Command::new(system_node())
        .arg(fixture("fake-dsh.cjs"))
        .arg("web")
        .arg("--port")
        .arg(port.to_string())
        .current_dir(work)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

async fn wait_dsh_ready(dsh_port: u16) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = get_direct(&format!("http://127.0.0.1:{dsh_port}/")).await {
            if r.status().is_success() {
                return;
            }
        }
        assert!(Instant::now() < deadline, "fixture dsh 15s 内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 与生产 lib.rs 同构：RemoteSettings 经 watch 通道喂给管理器；
/// ssh_exe/tunnel_exe 都传 fixture 脚本（node 跑 fake-ssh.cjs / fake-cloudflared.cjs），
/// ssh_work_dir 用 tempdir 隔离 fake-ssh.fail / fake-cloudflared.exit-after 标记文件。
fn make_manager(
    remote_port: u16,
    ssh: SshTunnelSettings,
    cloudflare: bool,
    dsh_port: Option<u16>,
) -> (
    RemoteManager,
    watch::Sender<RemoteSettings>,
    Arc<Mutex<Vec<RemoteStatus>>>,
) {
    let (ctx, crx) = watch::channel(RemoteSettings { port: remote_port, ssh, cloudflare });
    let (_dtx, drx) = watch::channel(dsh_port);
    let statuses: Arc<Mutex<Vec<RemoteStatus>>> = Arc::new(Mutex::new(Vec::new()));
    let st = statuses.clone();
    // fake 脚本的 current_dir 必须是真实存在的目录且测试期间不消失：
    // TempDir drop 会删目录，这里 leak 掉（测试进程生命周期内有效）
    let work_dir = Box::leak(Box::new(tempfile::tempdir().unwrap())).path().to_path_buf();
    let mgr = RemoteManager::new(
        Arc::new(TestPlatform),
        system_node(),
        vec![fixture("fake-ssh.cjs").to_string_lossy().into_owned()],
        work_dir.clone(),
        system_node(),
        vec![fixture("fake-cloudflared.cjs").to_string_lossy().into_owned()],
        crx,
        drx,
        Box::new(move |ev| {
            if let RemoteEvent::Status(s) = ev {
                st.lock().unwrap().push(s);
            }
        }),
    );
    (mgr, ctx, statuses)
}

fn ssh_off() -> SshTunnelSettings {
    SshTunnelSettings::default()
}

fn ssh_on(server: &str, expose_port: u16, link_port: u16) -> SshTunnelSettings {
    SshTunnelSettings {
        enabled: true,
        server: server.into(),
        ssh_port: 22,
        user: "root".into(),
        key_path: r"C:\keys\id_ed25519".into(),
        expose_port,
        link_port,
    }
}

#[test]
fn compose_link_appends_token() {
    assert_eq!(
        compose_link("http://192.168.1.5:7788", "abc123"),
        "http://192.168.1.5:7788/?token=abc123"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_with_port_in_use_errors() {
    // 占住一个端口，RemoteManager 绑 0.0.0.0:同端口必然失败
    let blocker = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
    let port = blocker.local_addr().unwrap().port();
    let (mgr, _ctx, _statuses) = make_manager(port, ssh_off(), false, None);
    let s = mgr.start().await;
    assert_eq!(s.phase, "error");
    let err = s.error.unwrap();
    assert!(err.contains("占用"), "错误应提示端口占用：{err}");
    assert!(s.link.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_chain_up_then_off() {
    // fixture dsh + 局域网直连（无 SSH）：鉴权门岗 → 302 种 cookie → 放行 → 幂等 → 关停
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let remote_port = free_port().unwrap();
    let (mgr, _ctx, _statuses) = make_manager(remote_port, ssh_off(), false, Some(dsh_port));
    let s = mgr.start().await;
    assert_eq!(s.phase, "up");
    let link = s.link.as_deref().unwrap().to_string();
    assert!(link.starts_with("http://"), "局域网链接应以 http:// 开头：{link}");
    assert!(
        link.contains(&format!(":{remote_port}/?token=")),
        "链接应含固定端口与 token：{link}"
    );
    let token = link.rsplit("?token=").next().unwrap().to_string();

    // 复现浏览器首次点击（0.0.0.0 绑定含回环，经 127.0.0.1 可达）：
    // 带 token 访问 → 302 + cookie → 带 cookie 拿到 dsh 内容
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // 测试环境可能设了系统代理（HTTP_PROXY），访问 127.0.0.1 必须直连
        .no_proxy()
        .build()
        .unwrap();
    let r = http
        .get(format!("http://127.0.0.1:{remote_port}/?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302);
    let r = http
        .get(format!("http://127.0.0.1:{remote_port}/"))
        .header("cookie", format!("__dsh_remote={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "ok");

    // 幂等 start：不改变 token/链接
    let s2 = mgr.start().await;
    assert_eq!(
        s2.link.as_deref(),
        Some(link.as_str()),
        "重复 start 不应换 token"
    );

    let s3 = mgr.stop().await;
    assert_eq!(s3.phase, "off");
    assert!(s3.link.is_none());
    // stop 后端口应已释放（换新客户端：旧 keep-alive 连接复用成功不代表在服务）
    let deadline = Instant::now() + Duration::from_secs(5);
    let refused = loop {
        match http
            .get(format!("http://127.0.0.1:{remote_port}/"))
            .header("cookie", format!("__dsh_remote={token}"))
            .send()
            .await
        {
            Err(_) => break true,
            Ok(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Ok(_) => break false,
        }
    };
    assert!(refused, "stop 后端口不应再响应新连接");
    let rebind = std::net::TcpListener::bind(("0.0.0.0", remote_port));
    assert!(rebind.is_ok(), "stop 后端口应可重新绑定：{rebind:?}");
    drop(rebind);

    let _ = dsh.kill();
}

/// SSH 隧道模式：链接地址 = 服务器地址 + 暴露端口（与设置一致），本地代理仍可直连
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ssh_tunnel_up_uses_server_url() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let remote_port = free_port().unwrap();
    let (mgr, _ctx, _statuses) =
        make_manager(remote_port, ssh_on("vps.example.com", 8080, 0), false, Some(dsh_port));
    mgr.start().await;
    // SSH 隧道就绪是异步的（监督循环先确认进程稳定存活再 Up），轮询等待
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let s = mgr.status();
        if s.phase == "up" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "SSH 隧道未在 20s 内 up，当前：{:?}",
            mgr.status()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let s = mgr.status();
    let link = s.link.unwrap();
    assert!(
        link.starts_with("http://vps.example.com:8080/?token="),
        "SSH 模式链接应使用服务器地址：{link}"
    );
    // 本地代理照常工作（隧道出口连 127.0.0.1:port）
    let token = link.rsplit("?token=").next().unwrap().to_string();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .unwrap();
    let r = http
        .get(format!("http://127.0.0.1:{remote_port}/?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302);

    mgr.stop().await;
    let _ = dsh.kill();
}

/// SSH 隧道 + 反向代理：https 前缀 + link_port 覆盖（对外端口 ≠ 转发端口）时，
/// 链接用 https 与覆盖端口，而 -R 仍绑定 expose_port
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ssh_link_port_override_for_reverse_proxy() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let remote_port = free_port().unwrap();
    // 服务器 8080 是 Nginx 反代监听口（-R 绑它），对外公布走 https://vps.example.com:8443
    let (mgr, _ctx, _statuses) = make_manager(
        remote_port,
        ssh_on("https://vps.example.com", 8080, 8443),
        false,
        Some(dsh_port),
    );
    mgr.start().await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let s = mgr.status();
        if s.phase == "up" {
            break;
        }
        assert!(Instant::now() < deadline, "SSH 隧道未在 20s 内 up：{:?}", mgr.status());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let link = mgr.status().link.unwrap();
    assert!(
        link.starts_with("https://vps.example.com:8443/?token="),
        "反向代理场景链接应为 https + 覆盖端口：{link}"
    );

    mgr.stop().await;
    let _ = dsh.kill();
}

/// SSH 失败路径：在 fake-ssh 工作目录写入 fake-ssh.fail，隧道 stderr 出现
/// "Permission denied" → RemoteManager 转 error，错误信息透出
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ssh_failure_via_marker() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let ssh_work = tempfile::tempdir().unwrap();
    std::fs::write(ssh_work.path().join("fake-ssh.fail"), "Permission denied (publickey).\n").unwrap();
    let remote_port = free_port().unwrap();
    let (ctx, crx) = watch::channel(RemoteSettings {
        port: remote_port,
        ssh: ssh_on("vps.example.com", 8080, 0),
        cloudflare: false,
    });
    let (_dtx, drx) = watch::channel(Some(dsh_port));
    let mgr = RemoteManager::new(
        Arc::new(TestPlatform),
        system_node(),
        vec![fixture("fake-ssh.cjs").to_string_lossy().into_owned()],
        ssh_work.path().to_path_buf(),
        system_node(),
        vec![fixture("fake-cloudflared.cjs").to_string_lossy().into_owned()],
        crx,
        drx,
        Box::new(|_| {}),
    );
    let _ = ctx;

    mgr.start().await;
    // 隧道失败在 8s 内上报（fake-ssh 立即打印错误退出）
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = mgr.status();
        if s.phase == "error" {
            let err = s.error.unwrap();
            assert!(
                err.contains("Permission denied") || err.contains("SSH 隧道失败"),
                "错误应透出 ssh 诊断：{err}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "SSH 失败未在 15s 内转 error，当前：{:?}", mgr.status());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    mgr.stop().await;
    let _ = dsh.kill();
}

/// 设置里改端口/SSH 配置（经 watch 通道 send）后，下次 start 用新配置
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn port_change_applies_on_next_start() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let port_a = free_port().unwrap();
    let (mgr, ctx, _statuses) = make_manager(port_a, ssh_off(), false, Some(dsh_port));
    let s = mgr.start().await;
    assert_eq!(s.phase, "up");
    assert!(s.link.unwrap().contains(&format!(":{port_a}/?token=")));

    let port_b = free_port().unwrap();
    ctx.send(RemoteSettings { port: port_b, ssh: ssh_off(), cloudflare: false }).unwrap();
    let s2 = mgr.stop().await;
    assert_eq!(s2.phase, "off");
    let s3 = mgr.start().await;
    assert_eq!(s3.phase, "up");
    assert!(
        s3.link.unwrap().contains(&format!(":{port_b}/?token=")),
        "改端口后应绑新端口"
    );

    mgr.stop().await;
    let _ = dsh.kill();
}

/// Cloudflare Quick Tunnel 模式：cloudflared 出站隧道把本地代理发布到公网
/// trycloudflare 域名；链接 = 隧道 URL + token，phase 经 Starting 转 Up。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloudflare_tunnel_up_uses_cf_url() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let remote_port = free_port().unwrap();
    let (mgr, _ctx, _statuses) = make_manager(remote_port, ssh_off(), true, Some(dsh_port));
    let s = mgr.start().await;
    // Cloudflare 是链接主宰：进入 Starting（等隧道就绪），再异步转 Up
    assert_eq!(s.phase, "starting");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = mgr.status();
        if s.phase == "up" {
            break;
        }
        assert!(Instant::now() < deadline, "cloudflared 未在 30s 内 up：{:?}", mgr.status());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let link = mgr.status().link.unwrap();
    assert!(
        link.starts_with("https://") && link.contains(".trycloudflare.com/?token="),
        "Cloudflare 模式链接应为隧道 URL：{link}"
    );

    mgr.stop().await;
    let _ = dsh.kill();
}

/// cloudflared 缺失：请求出站隧道但 exe 不是文件 → 直接转 error，提示重装/重跑 fetch-runtime
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloudflare_missing_bin_errors() {
    let dsh_port = free_port().unwrap();
    let (ctx, crx) = watch::channel(RemoteSettings {
        port: free_port().unwrap(),
        ssh: ssh_off(),
        cloudflare: true,
    });
    let (_dtx, drx) = watch::channel(Some(dsh_port));
    // tunnel_exe 指向不存在的路径
    let work_dir = Box::leak(Box::new(tempfile::tempdir().unwrap())).path().to_path_buf();
    let mgr = RemoteManager::new(
        Arc::new(TestPlatform),
        system_node(),
        vec![fixture("fake-ssh.cjs").to_string_lossy().into_owned()],
        work_dir.clone(),
        PathBuf::from(r"C:\nonexistent\cloudflared.exe"),
        vec![],
        crx,
        drx,
        Box::new(|_| {}),
    );
    let _ = ctx;
    let s = mgr.start().await;
    assert_eq!(s.phase, "error");
    let err = s.error.unwrap();
    assert!(err.contains("cloudflared 缺失"), "应提示 cloudflared 缺失：{err}");
    assert!(s.link.is_none());
}

/// Cloudflare 与 SSH 并存：两者都开时 Cloudflare 是链接主宰（cf_drives），
/// 链接取隧道 URL；SSH 隧道在后台跑、不抢占链接。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloudflare_drives_when_coexisting_with_ssh() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let remote_port = free_port().unwrap();
    let (mgr, _ctx, _statuses) =
        make_manager(remote_port, ssh_on("vps.example.com", 8080, 0), true, Some(dsh_port));
    let s = mgr.start().await;
    assert_eq!(s.phase, "starting");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = mgr.status();
        if s.phase == "up" {
            break;
        }
        assert!(Instant::now() < deadline, "并存模式未在 30s 内 up：{:?}", mgr.status());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let link = mgr.status().link.unwrap();
    assert!(
        link.contains(".trycloudflare.com/?token="),
        "Cloudflare 主宰时链接应为隧道 URL（SSH 不抢占）：{link}"
    );

    mgr.stop().await;
    let _ = dsh.kill();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reset_link_requires_running() {
    let port = free_port().unwrap();
    let (mgr, _ctx, _statuses) = make_manager(port, ssh_off(), false, None);
    assert!(mgr.reset_link().is_err(), "off 态重置应报错");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reset_link_rotates_token_keeps_url() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut dsh = spawn_fixture_dsh(dsh_port, work.path());
    wait_dsh_ready(dsh_port).await;

    let remote_port = free_port().unwrap();
    let (mgr, _ctx, _statuses) = make_manager(remote_port, ssh_off(), false, Some(dsh_port));
    mgr.start().await;
    let s = mgr.status();
    let old_link = s.link.unwrap();
    let old_token = old_link.rsplit("?token=").next().unwrap().to_string();

    // 重置：token 轮换、端口与地址不变
    let s2 = mgr.reset_link().expect("Up 态重置应成功");
    assert_eq!(s2.phase, "up");
    let new_link = s2.link.unwrap();
    assert_eq!(
        new_link.split("?token=").next(),
        old_link.split("?token=").next(),
        "重置不应换端口/地址"
    );
    let new_token = new_link.rsplit("?token=").next().unwrap().to_string();
    assert_ne!(old_token, new_token, "token 必须轮换");

    // 门岗即刻生效：旧凭据 403，新凭据放行
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .unwrap();
    let r = http
        .get(format!("http://127.0.0.1:{remote_port}/"))
        .header("cookie", format!("__dsh_remote={old_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "旧 cookie 应立即失效");
    let r = http
        .get(format!("http://127.0.0.1:{remote_port}/?token={new_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302, "新 token 应正常种 cookie");

    mgr.stop().await;
    let _ = dsh.kill();
}
