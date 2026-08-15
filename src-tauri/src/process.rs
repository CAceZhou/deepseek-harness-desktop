use crate::diagnostics::LogRing;
use crate::platform::Platform;
use crate::port::{free_port, wait_ready};
use crate::runtime::RuntimePaths;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_FAILURES: u32 = 5;
const READY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DshState {
    Starting,
    Ready { port: u16 },
    Failed(String),
    Stopped,
}

#[derive(Debug, Clone)]
pub enum ProcessEvent {
    StateChanged(DshState),
    Log(String),
}

struct Inner {
    platform: Arc<dyn Platform>,
    paths: RuntimePaths,
    state: Mutex<DshState>,
    pid: AtomicU32,
    log_ring: LogRing,
    on_event: Arc<dyn Fn(ProcessEvent) + Send + Sync>,
    shutdown: AtomicBool,
    stop: Notify,
    restart: Notify,
}

#[derive(Clone)]
pub struct DshProcess {
    inner: Arc<Inner>,
}

impl DshProcess {
    pub fn spawn_supervised(
        platform: Arc<dyn Platform>,
        paths: RuntimePaths,
        log_ring: LogRing,
        events: impl Fn(ProcessEvent) + Send + Sync + 'static,
    ) -> Self {
        let this = Self {
            inner: Arc::new(Inner {
                platform,
                paths,
                state: Mutex::new(DshState::Starting),
                pid: AtomicU32::new(0),
                log_ring,
                on_event: Arc::new(events),
                shutdown: AtomicBool::new(false),
                stop: Notify::new(),
                restart: Notify::new(),
            }),
        };
        let runner = this.clone();
        tokio::spawn(async move { runner.supervise_loop().await });
        this
    }

    pub fn state(&self) -> DshState {
        self.inner.state.lock().unwrap().clone()
    }

    pub fn port(&self) -> Option<u16> {
        match self.state() {
            DshState::Ready { port } => Some(port),
            _ => None,
        }
    }

    pub fn pid(&self) -> Option<u32> {
        match self.inner.pid.load(Ordering::SeqCst) {
            0 => None,
            p => Some(p),
        }
    }

    pub async fn stop(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        self.inner.stop.notify_one();
    }

    pub async fn restart(&self) {
        let alive = !matches!(self.state(), DshState::Failed(_) | DshState::Stopped);
        if alive {
            self.inner.restart.notify_one();
        } else {
            // 监督循环已退出（Failed/Stopped），重新拉起
            self.inner.shutdown.store(false, Ordering::SeqCst);
            let this = self.clone();
            tokio::spawn(async move { this.supervise_loop().await });
        }
    }

    async fn supervise_loop(&self) {
        let mut failures = 0u32;
        let mut backoff = Duration::from_millis(500);
        loop {
            if self.inner.shutdown.load(Ordering::SeqCst) {
                break;
            }
            self.set_state(DshState::Starting);
            let port = match free_port() {
                Ok(p) => p,
                Err(e) => {
                    self.set_state(DshState::Failed(format!("no free port: {e}")));
                    break;
                }
            };
            self.log(format!(
                "[dshdesktop] starting dsh web --port {port} (node={}, bin={}, cwd={})",
                self.inner.paths.node_exe.display(),
                self.inner.paths.dsh_bin.display(),
                self.inner.paths.work_dir.display()
            ));
            let mut cmd = Command::new(&self.inner.paths.node_exe);
            cmd.arg(&self.inner.paths.dsh_bin)
                .arg("web")
                .arg("--port")
                .arg(port.to_string())
                .env("DSH_HOME", &self.inner.paths.home)
                .current_dir(&self.inner.paths.work_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            self.inner.platform.configure_child_command(&mut cmd);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    self.set_state(DshState::Failed(format!("spawn failed: {e}")));
                    break;
                }
            };
            let pid = child.id().unwrap_or(0);
            self.inner.pid.store(pid, Ordering::SeqCst);
            if let Some(out) = child.stdout.take() {
                self.spawn_pump(out);
            }
            if let Some(err) = child.stderr.take() {
                self.spawn_pump(err);
            }

            if wait_ready(port, READY_TIMEOUT).await {
                failures = 0;
                backoff = Duration::from_millis(500);
                self.set_state(DshState::Ready { port });
            } else {
                self.log("[dshdesktop] dsh not ready within 60s, killing");
                self.inner.platform.kill_process_tree(pid);
                let _ = child.wait().await;
                self.inner.pid.store(0, Ordering::SeqCst);
                failures += 1;
                if failures >= MAX_FAILURES {
                    self.set_state(DshState::Failed("dsh failed to become ready".into()));
                    break;
                }
                if !self.wait_backoff(&mut backoff).await {
                    break;
                }
                continue;
            }

            tokio::select! {
                status = child.wait() => {
                    self.log(format!("[dshdesktop] dsh exited: {status:?}"));
                }
                _ = self.inner.stop.notified() => {
                    self.inner.platform.kill_process_tree(pid);
                    let _ = child.wait().await;
                    self.inner.pid.store(0, Ordering::SeqCst);
                    self.set_state(DshState::Stopped);
                    return;
                }
                _ = self.inner.restart.notified() => {
                    self.inner.platform.kill_process_tree(pid);
                    let _ = child.wait().await;
                    self.inner.pid.store(0, Ordering::SeqCst);
                    continue;
                }
            }
            self.inner.pid.store(0, Ordering::SeqCst);

            if self.inner.shutdown.load(Ordering::SeqCst) {
                self.set_state(DshState::Stopped);
                break;
            }
            failures += 1;
            if failures >= MAX_FAILURES {
                self.set_state(DshState::Failed("too many consecutive crashes".into()));
                break;
            }
            self.log(format!("[dshdesktop] restarting in {}ms", backoff.as_millis()));
            if !self.wait_backoff(&mut backoff).await {
                break;
            }
        }
    }

    /// 退避等待，期间响应 stop / restart。返回 false 表示收到 stop，循环应终止。
    async fn wait_backoff(&self, backoff: &mut Duration) -> bool {
        let current = *backoff;
        *backoff = (current * 2).min(MAX_BACKOFF);
        tokio::select! {
            _ = tokio::time::sleep(current) => true,
            _ = self.inner.restart.notified() => true,
            _ = self.inner.stop.notified() => {
                self.set_state(DshState::Stopped);
                false
            }
        }
    }

    fn spawn_pump<R: AsyncRead + Unpin + Send + 'static>(&self, reader: R) {
        let ring = self.inner.log_ring.clone();
        let emit = self.inner.on_event.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                ring.push_line(line.clone());
                emit(ProcessEvent::Log(line));
            }
        });
    }

    fn set_state(&self, s: DshState) {
        *self.inner.state.lock().unwrap() = s.clone();
        (self.inner.on_event)(ProcessEvent::StateChanged(s));
    }

    fn log(&self, line: impl Into<String>) {
        let line = line.into();
        self.inner.log_ring.push_line(line.clone());
        (self.inner.on_event)(ProcessEvent::Log(line));
    }
}
