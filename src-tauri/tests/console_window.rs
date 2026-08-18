use dshdesktop_lib::platform;
use std::process::Stdio;
use std::time::Duration;

/// 检测目标进程是否拥有“可见的”控制台窗口。
/// 注意：conhost.exe 进程的存在不代表窗口可见（CREATE_NO_WINDOW 会产生无窗口的
/// 隐藏控制台，后台服务普遍如此），因此必须枚举可见的 ConsoleWindowClass 窗口，
/// 其属主是 conhost.exe，再经 toolhelp 快照找到 conhost 的父进程。
#[cfg(windows)]
mod visible_console {
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
    };

    /// 所有可见 ConsoleWindowClass 窗口的属主进程（conhost.exe）PID
    fn visible_console_owner_pids() -> Vec<u32> {
        unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> i32 {
            if IsWindowVisible(hwnd) != 0 {
                let mut buf = [0u16; 256];
                let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "ConsoleWindowClass" {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(hwnd, &mut pid);
                    (*(lp as *mut Vec<u32>)).push(pid);
                }
            }
            1
        }
        let mut out: Vec<u32> = Vec::new();
        unsafe {
            EnumWindows(Some(cb), &mut out as *mut Vec<u32> as LPARAM);
        }
        out
    }

    /// pid → parent pid 映射
    fn parent_map() -> HashMap<u32, u32> {
        process_snapshot().0
    }

    /// pid → exe 文件名映射（如 "taskkill.exe"）
    fn exe_names() -> HashMap<u32, String> {
        process_snapshot().1
    }

    /// 一次 toolhelp 快照同时取回 (pid→parent, pid→exe 名) 两张表，保证二者一致
    fn process_snapshot() -> (HashMap<u32, u32>, HashMap<u32, String>) {
        let mut parents = HashMap::new();
        let mut names = HashMap::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap == -1isize as _ {
                return (parents, names);
            }
            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut pe) != 0 {
                loop {
                    parents.insert(pe.th32ProcessID, pe.th32ParentProcessID);
                    let len = pe
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(pe.szExeFile.len());
                    names.insert(
                        pe.th32ProcessID,
                        String::from_utf16_lossy(&pe.szExeFile[..len]),
                    );
                    if Process32NextW(snap, &mut pe) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
        }
        (parents, names)
    }

    pub fn has_visible_console(target_pid: u32) -> bool {
        let parents = parent_map();
        visible_console_owner_pids().iter().any(|&owner| {
            // 实测（Win10 22H2）：可见控制台窗口直接属主就是客户端进程本身；
            // 兼容旧模型：属主是 conhost.exe，其父进程才是客户端。
            owner == target_pid || parents.get(&owner) == Some(&target_pid)
        })
    }

    /// 是否存在属主链指向 taskkill.exe 且该 taskkill 是 parent_pid 直属子进程的
    /// 可见控制台窗口（kill_process_tree 闪窗回归测试的探针）
    pub fn has_visible_taskkill_console(parent_pid: u32) -> bool {
        let parents = parent_map();
        let names = exe_names();
        let is_taskkill =
            |pid: u32| names.get(&pid).is_some_and(|n| n.eq_ignore_ascii_case("taskkill.exe"));
        visible_console_owner_pids().iter().any(|&owner| {
            let client = if is_taskkill(owner) {
                Some(owner)
            } else {
                parents.get(&owner).copied().filter(|&p| is_taskkill(p))
            };
            client.is_some_and(|c| parents.get(&c) == Some(&parent_pid))
        })
    }
}

#[cfg(windows)]
fn node_path() -> String {
    let out = std::process::Command::new("where").arg("node").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    text.lines().next().unwrap().trim().to_string()
}

/// 对照组成立的前提：存在真实显示设备承载的可视桌面。
/// GitHub Actions 等 CI 恒设 CI=true，其 runner 以服务身份跑在 Session 0——
/// 该会话的 WinSta0 虽带 WSF_VISIBLE 标志，但不接任何显示设备，
/// 控制台窗口不可能出现在任何用户桌面上（实测 CI 上枚举也拿不到它）。
#[cfg(windows)]
fn control_group_possible() -> bool {
    if std::env::var_os("CI").is_some() {
        return false;
    }
    window_station_visible()
}

/// 进程窗口站是否带 WSF_VISIBLE（作为 CI 环境变量之外的兜底信号）。
#[cfg(windows)]
fn window_station_visible() -> bool {
    use windows_sys::Win32::System::StationsAndDesktops::{
        GetProcessWindowStation, GetUserObjectInformationW, USEROBJECTFLAGS, UOI_FLAGS,
    };
    const WSF_VISIBLE: u32 = 1; // windows-sys 未导出该常量
    unsafe {
        let hwinsta = GetProcessWindowStation();
        let mut flags: USEROBJECTFLAGS = std::mem::zeroed();
        let mut needed = 0u32;
        if GetUserObjectInformationW(
            hwinsta,
            UOI_FLAGS,
            &mut flags as *mut _ as *mut _,
            std::mem::size_of::<USEROBJECTFLAGS>() as u32,
            &mut needed,
        ) == 0
        {
            return false;
        }
        flags.dwFlags & WSF_VISIBLE != 0
    }
}

/// Windows 上子进程必须不携带可见控制台窗口（CREATE_NO_WINDOW）。
/// 回归测试：spawn 一个存活的 node 进程，断言它没有可见的控制台窗口。
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawned_child_has_no_visible_console_window() {
    let mut cmd = tokio::process::Command::new(node_path());
    cmd.arg("-e")
        .arg("setInterval(()=>{}, 1000)")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    platform::current().configure_child_command(&mut cmd);
    let mut child = cmd.spawn().unwrap();
    let pid = child.id().unwrap();

    // 给窗口一点创建时间
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let visible = visible_console::has_visible_console(pid);
    child.kill().await.unwrap();

    assert!(!visible, "带 CREATE_NO_WINDOW 的子进程不应有可见控制台窗口");
}

/// 对照组：CREATE_NEW_CONSOLE 必然产生可见控制台窗口。
/// 若此测试失败，说明上面的“无可见窗口”断言不可信（检测手段失效）。
/// 注意：运行该测试会在屏幕上短暂弹出一个控制台窗口，属正常现象。
/// 无头会话（CI runner 的 Session 0）没有真实显示设备，对照组前提不成立，显式跳过。
#[cfg(windows)]
#[test]
fn spawned_child_with_new_console_has_visible_window() {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x00000010;

    if !control_group_possible() {
        eprintln!("无头会话（CI/Session 0），跳过对照组");
        return;
    }

    let mut child = std::process::Command::new(node_path())
        .arg("-e")
        .arg("setInterval(()=>{}, 1000)")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .unwrap();
    let pid = child.id();

    std::thread::sleep(Duration::from_millis(1000));
    let visible = visible_console::has_visible_console(pid);
    let _ = child.kill();

    assert!(visible, "CREATE_NEW_CONSOLE 的子进程应有可见控制台窗口（对照组）");
}

/// 回归：kill_process_tree 起 taskkill 不得弹出可见控制台窗口（退出/重启时闪 cmd）。
/// taskkill 是控制台子系统程序；DSHDesktop.exe 是 GUI 程序没有控制台，spawn 不带
/// CREATE_NO_WINDOW 时系统会为 taskkill 新分配一个可见控制台窗口。
/// 注意复现环境必须是"完全没有控制台"的父进程：CREATE_NO_WINDOW 只是隐藏控制台
/// （子进程会静默继承，不产生新窗口），所以子分支先 FreeConsole() 挣脱继承来的
/// 隐藏控制台，等价于 GUI 程序的环境；父进程侧轮询抓窗口。
#[cfg(windows)]
#[test]
fn kill_process_tree_spawns_no_visible_console_window() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const CHILD_ENV: &str = "DSHDESKTOP_KILL_TREE_CHILD";

    if std::env::var_os(CHILD_ENV).is_some() {
        // 挣脱隐藏控制台，成为真正无控制台进程（等价 DSHDesktop.exe 的 GUI 环境）
        unsafe { windows_sys::Win32::System::Console::FreeConsole() };
        // 连续杀多棵树，拉长 taskkill 窗口的存在时间便于父进程捕获
        for _ in 0..30 {
            let sleeper = std::process::Command::new("ping.exe")
                .args(["-n", "100", "127.0.0.1"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .expect("spawn sleeper");
            platform::current().kill_process_tree(sleeper.id());
        }
        // 给父进程轮询留余量
        std::thread::sleep(Duration::from_millis(500));
        return;
    }

    let exe = std::env::current_exe().unwrap();
    let mut child = std::process::Command::new(exe)
        .arg("kill_process_tree_spawns_no_visible_console_window")
        .args(["--exact", "--nocapture"])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .expect("spawn console-less copy of test binary");
    let child_pid = child.id();

    let mut flashed = false;
    loop {
        if visible_console::has_visible_taskkill_console(child_pid) {
            flashed = true;
            break;
        }
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let status = child.wait().expect("wait");
    assert!(status.success(), "无控制台子分支应正常跑完");
    assert!(
        !flashed,
        "kill_process_tree 不得让 taskkill 弹出可见控制台窗口（退出闪 cmd）"
    );
}
