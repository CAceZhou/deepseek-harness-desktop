use std::io;
use std::net::TcpListener;
use std::time::Duration;

/// 让 OS 分配一个空闲端口。返回后到实际使用之间存在小竞态窗口，
/// 调用方需在启动失败时重试另一个端口。
pub fn free_port() -> io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

/// 轮询 http://127.0.0.1:port/ 直到拿到任意 HTTP 响应或超时。
pub async fn wait_ready(port: u16, timeout: Duration) -> bool {
    let url = format!("http://127.0.0.1:{port}/");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap_or_default();
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if client.get(&url).send().await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::thread;

    #[test]
    fn free_port_returns_bindable_port() {
        let p = free_port().unwrap();
        assert!(TcpListener::bind(("127.0.0.1", p)).is_ok());
    }

    fn spawn_mini_http() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf);
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");
                }
            }
        });
        port
    }

    #[tokio::test]
    async fn wait_ready_true_when_http_responds() {
        let port = spawn_mini_http();
        assert!(wait_ready(port, Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn wait_ready_false_on_timeout() {
        // 绑定但不 accept：端口被占用、TCP 能连上但没有 HTTP 响应，确定性触发超时
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!wait_ready(port, Duration::from_millis(600)).await);
    }
}
