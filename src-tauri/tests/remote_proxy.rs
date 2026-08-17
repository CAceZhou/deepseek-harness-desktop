use dshdesktop_lib::port::free_port;
use dshdesktop_lib::remote::proxy::{spawn_proxy, ProxyHandle, COOKIE_NAME};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

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

fn spawn_fixture(port: u16, work: &Path) -> std::process::Child {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake-dsh.cjs");
    Command::new(system_node())
        .arg(fixture)
        .arg("web")
        .arg("--port")
        .arg(port.to_string())
        .current_dir(work)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// 起代理指向 dsh_port（None 模拟 dsh 未就绪），返回 (句柄, token)
async fn start_proxy(dsh_port: Option<u16>) -> (ProxyHandle, Arc<str>) {
    let token: Arc<str> = dshdesktop_lib::remote::generate_token().into();
    let (_tx, rx) = watch::channel(dsh_port);
    let handle = spawn_proxy(token.clone(), rx, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    (handle, token)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // 测试环境可能设了系统代理（HTTP_PROXY），访问 127.0.0.1 必须直连
        .no_proxy()
        .build()
        .unwrap()
}

async fn get_direct(url: &str) -> reqwest::Result<reqwest::Response> {
    client().get(url).send().await
}

#[test]
fn token_is_64_hex_and_unique() {
    let a = dshdesktop_lib::remote::generate_token();
    let b = dshdesktop_lib::remote::generate_token();
    assert_eq!(a.len(), 64);
    assert!(
        a.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "应为小写 hex：{a}"
    );
    assert_ne!(a, b, "两次生成不应相同");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gate_requires_token() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut child = spawn_fixture(dsh_port, work.path());
    // 等 fixture 就绪
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = get_direct(&format!("http://127.0.0.1:{dsh_port}/")).await {
            if r.status().is_success() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "fixture 15s 内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let (proxy, token) = start_proxy(Some(dsh_port)).await;
    let base = format!("http://127.0.0.1:{}", proxy.port);
    let http = client();

    // 1. 无凭据 → 403
    let r = http.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(r.status(), 403, "无凭据应 403");

    // 2. 错误 token → 403，且有 ≥400ms 的防爆破延迟
    let t0 = Instant::now();
    let r = http
        .get(format!("{base}/?token=wrong"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "错误 token 应 403");
    assert!(
        t0.elapsed() >= Duration::from_millis(400),
        "错误 token 应有延迟"
    );

    // 3. 正确 token → 302 到去 token 的地址 + 种 cookie
    let r = http
        .get(format!("{base}/some/path?a=1&token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302, "正确 token 应 302，实际 {:?}", r.status());
    let loc = r
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(loc, "/some/path?a=1", "Location 应剥离 token，实际 {loc}");
    let set_cookie = r
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        set_cookie.contains(&format!("{COOKIE_NAME}={token}")),
        "应种 cookie，实际 {set_cookie}"
    );
    assert!(set_cookie.contains("HttpOnly"), "cookie 应 HttpOnly");

    // 4. 带 cookie → 200 且转发到 dsh
    let r = http
        .get(format!("{base}/"))
        .header("cookie", format!("{COOKIE_NAME}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "带 cookie 应放行，实际 {:?}", r.status());
    assert_eq!(r.text().await.unwrap(), "ok");

    let _ = child.kill();
    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dsh_down_returns_503() {
    let (proxy, token) = start_proxy(None).await;
    let http = client();
    let r = http
        .get(format!("http://127.0.0.1:{}/", proxy.port))
        .header("cookie", format!("{COOKIE_NAME}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503, "dsh 未就绪应 503，实际 {:?}", r.status());
    proxy.shutdown().await;
}

/// 真实 dsh 有浏览器信任栅栏：/api 请求若 Origin.host ≠ Host 头（或
/// sec-fetch-site: cross-site）→ 403。经隧道远程访问时浏览器带的是
/// trycloudflare 域名的 Origin，代理转发前必须剥掉这些浏览器标记头，
/// 否则页面上所有 RPC 调用（agentPreset.list/settings.describe/…）全 403。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forward_strips_browser_marker_headers() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut child = spawn_fixture(dsh_port, work.path());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = get_direct(&format!("http://127.0.0.1:{dsh_port}/")).await {
            if r.status().is_success() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "fixture 15s 内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let (proxy, token) = start_proxy(Some(dsh_port)).await;
    let base = format!("http://127.0.0.1:{}", proxy.port);
    let r = client()
        .post(format!("{base}/api/agentPreset.list"))
        .header("cookie", format!("{COOKIE_NAME}={token}"))
        .header("content-type", "application/json")
        // 模拟经隧道访问的浏览器：同源 POST 自动带 Origin/Referer/Sec-Fetch-*
        .header("origin", "https://random-sub.trycloudflare.com")
        .header("referer", "https://random-sub.trycloudflare.com/")
        .header("sec-fetch-site", "same-origin")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-dest", "empty")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "带隧道 Origin 的 RPC 经代理后不应触发 dsh 栅栏，实际 {:?}",
        r.status()
    );

    let _ = child.kill();
    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_bridged_with_cookie() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut child = spawn_fixture(dsh_port, work.path());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = get_direct(&format!("http://127.0.0.1:{dsh_port}/")).await {
            if r.status().is_success() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "fixture 15s 内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let (proxy, token) = start_proxy(Some(dsh_port)).await;
    let url = format!("ws://127.0.0.1:{}/api/events.mux", proxy.port);
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert("cookie", format!("{COOKIE_NAME}={token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("带 cookie 的 WS 握手应成功");

    use futures::StreamExt;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut got_heartbeat = false;
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) if t.as_str().contains("heartbeat") => {
                got_heartbeat = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            other => panic!("WS 读帧异常：{other:?}"),
        }
    }

    let _ = child.kill();
    proxy.shutdown().await;
    assert!(got_heartbeat, "15s 内应经代理收到 heartbeat 帧");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_rejected_without_cookie() {
    let (proxy, _token) = start_proxy(None).await;
    let url = format!("ws://127.0.0.1:{}/api/events.mux", proxy.port);
    let req = url.into_client_request().unwrap();
    let result = tokio_tungstenite::connect_async(req).await;
    assert!(result.is_err(), "无 cookie 的 WS 握手应被拒");
    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_stops_listener() {
    let (proxy, _token) = start_proxy(Some(1)).await;
    let url = format!("http://127.0.0.1:{}/", proxy.port);
    let r = get_direct(&url).await.unwrap();
    assert_eq!(r.status(), 403);
    let port = proxy.port;
    proxy.shutdown().await;
    // 端口应已释放，可自行 bind（立即 drop 掉，否则后续请求会连到这个探针上干等）
    let rebind = std::net::TcpListener::bind(format!("127.0.0.1:{port}"));
    assert!(rebind.is_ok(), "shutdown 后端口应释放：{rebind:?}");
    drop(rebind);
    // 新连接应被拒（须用绕开系统代理的客户端，否则请求会被代理软件接管）
    let res = get_direct(&url).await;
    assert!(res.is_err(), "shutdown 后新连接应被拒，实际 {res:?}");
}

/// 重置链接（token 轮换）：旧 token 与旧 cookie 立即失效，新 token 正常种 cookie。
/// 代理与隧道都不重启，端口不变。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reset_token_revokes_old_credential() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut child = spawn_fixture(dsh_port, work.path());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = get_direct(&format!("http://127.0.0.1:{dsh_port}/")).await {
            if r.status().is_success() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "fixture 15s 内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let (proxy, old_token) = start_proxy(Some(dsh_port)).await;
    let base = format!("http://127.0.0.1:{}", proxy.port);
    let http = client();
    let old_port = proxy.port;

    // 重置前：旧凭据可用
    let r = http
        .get(format!("{base}/"))
        .header("cookie", format!("{COOKIE_NAME}={old_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "重置前旧 cookie 应放行");

    let new_token: Arc<str> = dshdesktop_lib::remote::generate_token().into();
    proxy.reset_token(new_token.clone());
    assert_eq!(proxy.port, old_port, "重置不应重启代理（端口不变）");

    // 旧 cookie → 403
    let r = http
        .get(format!("{base}/"))
        .header("cookie", format!("{COOKIE_NAME}={old_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "重置后旧 cookie 应失效");

    // 旧链接里的 token → 403
    let r = http
        .get(format!("{base}/?token={old_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "重置后旧 token 应失效");

    // 新 token → 302 + 种新 cookie
    let r = http
        .get(format!("{base}/?token={new_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302, "新 token 应 302 种 cookie");
    let r = http
        .get(format!("{base}/"))
        .header("cookie", format!("{COOKIE_NAME}={new_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "新 cookie 应放行");

    let _ = child.kill();
    proxy.shutdown().await;
}

/// 重置链接必须掐断已建立的 WS 桥接——否则链接泄露时攻击者已开的页面
/// 仍能持续收事件流，重置形同虚设。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reset_token_drops_live_ws() {
    let dsh_port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut child = spawn_fixture(dsh_port, work.path());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(r) = get_direct(&format!("http://127.0.0.1:{dsh_port}/")).await {
            if r.status().is_success() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "fixture 15s 内未就绪");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let (proxy, token) = start_proxy(Some(dsh_port)).await;
    let url = format!("ws://127.0.0.1:{}/api/events.mux", proxy.port);
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert("cookie", format!("{COOKIE_NAME}={token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("带 cookie 的 WS 握手应成功");

    use futures::StreamExt;
    // 确认桥接已通（能收到 fixture 心跳）
    let first = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
    assert!(matches!(first, Ok(Some(Ok(_)))), "桥接应已建立：{first:?}");

    let new_token: Arc<str> = dshdesktop_lib::remote::generate_token().into();
    proxy.reset_token(new_token);

    // 桥接被掐断：读侧应在 5s 内收到 Close 或连接错误/EOF
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "重置后已建立的 WS 应在 5s 内被掐断");

    let _ = child.kill();
    proxy.shutdown().await;
}

