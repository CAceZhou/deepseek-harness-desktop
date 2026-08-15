use super::{summarize, EventFilter, Notification, NotifySink, NotifySource};
use futures::future::BoxFuture;
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// 订阅 dsh 的事件下行流：WebSocket ws://127.0.0.1:<port>/api/events.mux
/// （dsh 的浏览器信任栅栏允许 loopback + 无 Origin 的请求，Rust 客户端天然满足）。
/// 断线自动重连，dsh 重启换端口后跟随新端口。
pub struct WsSource {
    pub filter: EventFilter,
}

impl NotifySource for WsSource {
    fn run(
        self: Box<Self>,
        sink: NotifySink,
        mut port: watch::Receiver<Option<u16>>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            loop {
                let Some(p) = *port.borrow() else {
                    // dsh 尚未就绪，等端口出现
                    if port.changed().await.is_err() {
                        return;
                    }
                    continue;
                };
                let url = format!("ws://127.0.0.1:{p}/api/events.mux");
                let ws = match tokio_tungstenite::connect_async(&url).await {
                    Ok((stream, _)) => stream,
                    Err(_) => {
                        tokio::select! {
                            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
                            _ = port.changed() => {}
                        }
                        continue;
                    }
                };
                let mut stream = ws;
                loop {
                    tokio::select! {
                        msg = stream.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if self.filter.matches(&text) {
                                        sink(Notification {
                                            title: "DSHDesktop".into(),
                                            body: summarize(&text),
                                        });
                                    }
                                }
                                // 连接关闭/出错/结束：重连
                                _ => break,
                            }
                        }
                        changed = port.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            break; // 端口变了，用新端口重连
                        }
                    }
                }
            }
        })
    }
}
