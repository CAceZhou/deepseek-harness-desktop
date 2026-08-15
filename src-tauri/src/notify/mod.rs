use futures::future::BoxFuture;
use regex::Regex;
use std::sync::Arc;
use tokio::sync::watch;

pub mod ws;

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

pub type NotifySink = Arc<dyn Fn(Notification) + Send + Sync>;

/// 通知来源适配器接口。dsh 上游接口不稳定（开发者预览），
/// 后期可加 FileWatchSource（解析 session jsonl）等替代实现。
pub trait NotifySource: Send {
    /// port 通过 watch 通道下发：dsh 每次 Ready（含重启换端口）都会更新。
    fn run(self: Box<Self>, sink: NotifySink, port: watch::Receiver<Option<u16>>) -> BoxFuture<'static, ()>;
}

/// 事件过滤器：匹配 WS 帧 JSON 中的 method 字段（server-request 帧的事件类型），
/// 只放行需要用户关注的事件：approval/requested（待批准）、question/requested（待回答）。
pub struct EventFilter {
    pattern: Regex,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self {
            pattern: Regex::new(
                r#""(method|type)"\s*:\s*"(approval/requested|question/requested)""#,
            )
            .unwrap(),
        }
    }
}

impl EventFilter {
    pub fn matches(&self, data: &str) -> bool {
        self.pattern.is_match(data)
    }
}

/// 尽力从事件帧 JSON 提取可读摘要（dsh 帧结构：{"type":"server-request","method":..,"payload":..}）。
pub fn summarize(frame: &str) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(frame).ok();
    let method = parsed
        .as_ref()
        .and_then(|v| v.get("method"))
        .and_then(|t| t.as_str())
        .map(String::from);
    match method.as_deref() {
        Some("approval/requested") => "dsh 有一个操作等待你批准".to_string(),
        Some("question/requested") => "dsh 有一个问题等待你回答".to_string(),
        Some(m) => format!("dsh 事件：{m}"),
        None => frame.chars().take(80).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_matches_method_keywords() {
        let f = EventFilter::default();
        assert!(f.matches(r#"{"type":"server-request","rpcId":"x","method":"approval/requested","payload":{}}"#));
        assert!(f.matches(r#"{"type":"server-request","method":"question/requested","payload":{}}"#));
        assert!(!f.matches(r#"{"type":"server-request","method":"host/session-added","payload":{}}"#));
        assert!(!f.matches("garbage"));
    }

    #[test]
    fn summarize_extracts_method() {
        assert_eq!(
            summarize(r#"{"type":"server-request","method":"approval/requested","payload":{}}"#),
            "dsh 有一个操作等待你批准"
        );
        assert_eq!(summarize("not json"), "not json");
    }
}
