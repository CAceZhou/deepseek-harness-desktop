use crate::process::DshProcess;
use crate::runtime::RuntimePaths;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use tauri::Url;

const LOG_CAPACITY: usize = 500;

/// dsh 输出日志的环形缓冲，容量 500 行，供诊断面板回填与事件订阅。
#[derive(Clone, Default)]
pub struct LogRing(Arc<RwLock<VecDeque<String>>>);

impl LogRing {
    pub fn push_line(&self, line: String) {
        let mut g = self.0.write().unwrap();
        if g.len() >= LOG_CAPACITY {
            g.pop_front();
        }
        g.push_back(line);
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.0.read().unwrap().iter().cloned().collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusDto {
    pub state: String,
    pub port: Option<u16>,
    pub pid: Option<u32>,
    pub version: String,
}

/// 应用级共享状态，注册为 Tauri managed state。
pub struct SharedState {
    pub process: DshProcess,
    pub log_ring: LogRing,
    pub runtime: RuntimePaths,
    pub version: String,
    pub home_url: Url,
    /// 是否首次启动（ensure_runtime 之前 dsh-home 不存在）；供启动画面决定是否显示进度条
    pub first_launch: bool,
}

/// 启动引导信息：无论运行时是否就绪都会注册，
/// 供前端启动画面主动查询（事件可能早于前端 listen 而丢失）。
#[derive(Default)]
pub struct BootstrapInfo(pub std::sync::Mutex<Option<String>>);

impl BootstrapInfo {
    pub fn set_error(&self, msg: String) {
        *self.0.lock().unwrap() = Some(msg);
    }
    pub fn error(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_ring_evicts_oldest_beyond_capacity() {
        let ring = LogRing::default();
        for i in 0..510 {
            ring.push_line(format!("line {i}"));
        }
        let snap = ring.snapshot();
        assert_eq!(snap.len(), LOG_CAPACITY);
        assert_eq!(snap[0], "line 10");
        assert_eq!(snap[LOG_CAPACITY - 1], "line 509");
    }
}
