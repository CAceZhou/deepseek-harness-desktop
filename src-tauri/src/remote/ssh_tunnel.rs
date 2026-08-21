//! 系统 OpenSSH 客户端的反向隧道（`ssh -R`）监督：把本地鉴权代理的固定端口
//! 转发到自建公网服务器，公网/异地经 `http://<server>:<expose_port>` 访问。
//! 就绪信号：进程稳定存活 UP_TIMEOUT 且 stderr 未出现错误关键字；错误/崩溃
//! 退避重启（配置性错误如密钥拒绝，累计 MAX_FAILURES 后终态 Failed）。
//!
//! 测试注入：`exe` 传 fixture 脚本路径（node 跑 fake-ssh.cjs），生产为系统
//! OpenSSH（C:\Windows\System32\OpenSSH\ssh.exe，Win10 1809+ 自带）。

use crate::platform::Platform;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

const UP_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_FAILURES: u32 = 5;

/// stderr 里出现即判定失败的错误关键字（鉴权/端口转发/网络问题）。
/// 匹配到后杀进程退避重启；多次失败说明是配置问题，终态 Failed 提示用户。
const ERROR_MARKERS: &[&str] = &[
    "Permission denied",
    "remote port forwarding failed",
    "Could not resolve hostname",
    "Connection refused",
    "Connection timed out",
    "Host key verification failed",
    "no such identity",
    "Load key",
    "Too many authentication failures",
    "Address already in use",
    "Network is unreachable",
    "operation timed out",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshState {
    Starting,
    Up,
    Failed(String),
    Stopped,
}

#[derive(Debug, Clone)]
pub enum SshEvent {
    StateChanged(SshState),
    Log(String),
}

struct Inner {
    platform: Arc<dyn Platform>,
    exe: PathBuf,
    /// 测试注入：exe=node，prefix_args=[fake-ssh.cjs]；生产为空（exe=ssh.exe）
    prefix_args: Vec<String>,
    args: Vec<String>,
    work_dir: PathBuf,
    state: Mutex<SshState>,
    pid: AtomicU32,
    on_event: Box<dyn Fn(SshEvent) + Send + Sync>,
    shutdown: AtomicBool,
    stop: Notify,
    /// stderr 泵命中错误关键字时通知监督循环处理
    failed: Notify,
    last_error: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct SshTunnelProcess {
    inner: Arc<Inner>,
}

impl SshTunnelProcess {
    /// 组装并拉起 ssh -N 反向隧道（参数见 build_args 注释）。
    /// 测试注入：exe=node、prefix_args=[fake-ssh.cjs]；生产 exe=系统 ssh、prefix 为空。
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_supervised(
        platform: Arc<dyn Platform>,
        exe: PathBuf,
        prefix_args: Vec<String>,
        ssh_port: u16,
        user: &str,
        server: &str,
        key_path: &Path,
        expose_port: u16,
        local_port: u16,
        work_dir: PathBuf,
        events: impl Fn(SshEvent) + Send + Sync + 'static,
    ) -> Self {
        let args = build_args(ssh_port, user, server, key_path, expose_port, local_port);

        let this = Self {
            inner: Arc::new(Inner {
                platform,
                exe,
                prefix_args,
                args,
                work_dir,
                state: Mutex::new(SshState::Starting),
                pid: AtomicU32::new(0),
                on_event: Box::new(events),
                shutdown: AtomicBool::new(false),
                stop: Notify::new(),
                failed: Notify::new(),
                last_error: Mutex::new(None),
            }),
        };
        let runner = this.clone();
        tokio::spawn(async move { runner.supervise_loop().await });
        this
    }

    pub fn state(&self) -> SshState {
        self.inner.state.lock().unwrap().clone()
    }

    pub async fn stop(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        self.inner.stop.notify_one();
    }

    async fn supervise_loop(&self) {
        let mut failures = 0u32;
        let mut backoff = Duration::from_millis(500);
        loop {
            if self.inner.shutdown.load(Ordering::SeqCst) {
                break;
            }
            self.set_state(SshState::Starting);
            self.log(format!(
                "[dshdesktop] starting ssh tunnel: {} {}",
                self.inner.exe.display(),
                self.inner.args.join(" ")
            ));
            let mut cmd = Command::new(&self.inner.exe);
            cmd.args(&self.inner.prefix_args)
                .args(&self.inner.args)
                .current_dir(&self.inner.work_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            self.inner.platform.configure_child_command(&mut cmd);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    self.set_state(SshState::Failed(format!("ssh 启动失败：{e}")));
                    break;
                }
            };
            let pid = child.id().unwrap_or(0);
            // 挂进 KILL_ON_JOB_CLOSE Job：本进程被强杀时 ssh 由内核连带回收
            self.inner.platform.register_child(pid);
            self.inner.pid.store(pid, Ordering::SeqCst);
            if let Some(out) = child.stdout.take() {
                self.spawn_pump(out, false);
            }
            if let Some(err) = child.stderr.take() {
                self.spawn_pump(err, true);
            }

            // 阶段一：稳定存活无错（UP_TIMEOUT）→ Up；错误/退出 → 计数退避重启
            tokio::select! {
                _ = self.inner.failed.notified() => {
                    self.kill_and_wait(pid, &mut child).await;
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        let msg = self.inner.last_error.lock().unwrap().clone()
                            .unwrap_or_else(|| "ssh tunnel failed repeatedly".into());
                        self.set_state(SshState::Failed(msg));
                        break;
                    }
                    if !self.wait_backoff(&mut backoff).await { break; }
                    continue;
                }
                status = child.wait() => {
                    self.log(format!("[dshdesktop] ssh exited before up: {status:?}"));
                    self.inner.pid.store(0, Ordering::SeqCst);
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        self.set_state(SshState::Failed("ssh tunnel exited repeatedly".into()));
                        break;
                    }
                    if !self.wait_backoff(&mut backoff).await { break; }
                    continue;
                }
                _ = tokio::time::sleep(UP_TIMEOUT) => {
                    failures = 0;
                    backoff = Duration::from_millis(500);
                    self.set_state(SshState::Up);
                }
                _ = self.inner.stop.notified() => {
                    self.kill_and_wait(pid, &mut child).await;
                    self.set_state(SshState::Stopped);
                    return;
                }
            }

            // 阶段二：稳态守进程退出/错误
            tokio::select! {
                status = child.wait() => {
                    self.log(format!("[dshdesktop] ssh exited: {status:?}"));
                }
                _ = self.inner.failed.notified() => {
                    self.log("[dshdesktop] ssh reported an error while up");
                }
                _ = self.inner.stop.notified() => {
                    self.kill_and_wait(pid, &mut child).await;
                    self.set_state(SshState::Stopped);
                    return;
                }
            }
            self.inner.pid.store(0, Ordering::SeqCst);

            if self.inner.shutdown.load(Ordering::SeqCst) {
                self.set_state(SshState::Stopped);
                break;
            }
            failures += 1;
            if failures >= MAX_FAILURES {
                self.set_state(SshState::Failed("too many consecutive tunnel crashes".into()));
                break;
            }
            self.log(format!("[dshdesktop] restarting ssh tunnel in {}ms", backoff.as_millis()));
            if !self.wait_backoff(&mut backoff).await {
                break;
            }
        }
    }

    async fn kill_and_wait(&self, pid: u32, child: &mut tokio::process::Child) {
        // 先直接 kill（tokio 内部 TerminateProcess，不需要外部 taskkill 权限），
        // 再杀树兜底（ssh 若派生子进程；失败忽略，进程可能已退出）
        let _ = child.kill().await;
        let _ = child.wait().await;
        self.inner.platform.kill_process_tree(pid);
        self.inner.pid.store(0, Ordering::SeqCst);
    }

    /// 退避等待，期间响应 stop。返回 false 表示收到 stop，循环应终止。
    async fn wait_backoff(&self, backoff: &mut Duration) -> bool {
        let current = *backoff;
        *backoff = (current * 2).min(MAX_BACKOFF);
        tokio::select! {
            _ = tokio::time::sleep(current) => true,
            _ = self.inner.stop.notified() => {
                self.set_state(SshState::Stopped);
                false
            }
        }
    }

    fn spawn_pump<R: AsyncRead + Unpin + Send + 'static>(&self, reader: R, is_stderr: bool) {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if is_stderr {
                    if ERROR_MARKERS.iter().any(|m| line.contains(*m)) {
                        *inner.last_error.lock().unwrap() = Some(line.clone());
                        (inner.on_event)(SshEvent::Log(format!(
                            "[dshdesktop] ssh error: {line}"
                        )));
                        inner.failed.notify_one();
                        continue;
                    }
                }
                (inner.on_event)(SshEvent::Log(line));
            }
        });
    }

    fn set_state(&self, s: SshState) {
        *self.inner.state.lock().unwrap() = s.clone();
        (self.inner.on_event)(SshEvent::StateChanged(s));
    }

    fn log(&self, line: impl Into<String>) {
        (self.inner.on_event)(SshEvent::Log(line.into()));
    }
}

/// 组装 ssh -N 反向隧道命令行参数（纯函数，供单元测试锚定参数形态）：
/// `ssh -N -T -o BatchMode=yes -o ExitOnForwardFailure=yes
///  -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10
///  -o ServerAliveInterval=30 -o ServerAliveCountMax=3
///  -p <ssh_port> -i <key> -R 0.0.0.0:<expose_port>:127.0.0.1:<local_port> <user>@<server>`
///
/// - BatchMode=yes：禁止交互输入（密码登录直接失败），鉴权只走私钥/agent
/// - ExitOnForwardFailure=yes：暴露端口绑定失败（如被占用/服务器 GatewayPorts
///   未开导致非回环绑定被拒）时 ssh 直接退出，错误进 stderr 可被检测
/// - StrictHostKeyChecking=accept-new：首次连接自动信任新 host key
/// - `-R 0.0.0.0:port:...`：显式请求绑定服务器所有网卡；服务器 sshd 需
///   `GatewayPorts yes`（或客户端不指定绑定地址时服务器默认只绑 127.0.0.1，
///   公网访问不到）
pub(crate) fn build_args(
    ssh_port: u16,
    user: &str,
    server: &str,
    key_path: &Path,
    expose_port: u16,
    local_port: u16,
) -> Vec<String> {
    let mut args = vec![
        "-N".into(),
        "-T".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "ServerAliveInterval=30".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
    ];
    args.push("-p".into());
    args.push(ssh_port.to_string());
    args.push("-i".into());
    args.push(key_path.to_string_lossy().into_owned());
    args.push("-R".into());
    args.push(format!("0.0.0.0:{expose_port}:127.0.0.1:{local_port}"));
    args.push(format!("{}@{}", user.trim(), server.trim()));
    args
}

#[cfg(test)]
mod tests {
    use super::build_args;
    use std::path::Path;

    #[test]
    fn args_shape_is_valid_ssh_invocation() {
        let args = build_args(2222, "root", "1.2.3.4", Path::new(r"C:\keys\id_ed25519"), 8080, 7788);
        assert!(args.contains(&"-R".into()));
        assert!(args.contains(&"0.0.0.0:8080:127.0.0.1:7788".into()));
        assert!(args.contains(&"root@1.2.3.4".into()));
        assert!(args.contains(&"-p".into()) && args.contains(&"2222".into()));
        assert!(args.contains(&"-i".into()) && args.contains(&r"C:\keys\id_ed25519".into()));
        assert!(args.contains(&"BatchMode=yes".into()), "必须禁交互，只走密钥鉴权");
        assert!(args.contains(&"ExitOnForwardFailure=yes".into()));
        assert!(args.contains(&"StrictHostKeyChecking=accept-new".into()));
    }
}
