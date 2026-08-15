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
        let mut map = HashMap::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap == -1isize as _ {
                return map;
            }
            let mut pe: PROCESSENTRY32W = std::mem::zeroed();
            pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut pe) != 0 {
                loop {
                    map.insert(pe.th32ProcessID, pe.th32ParentProcessID);
                    if Process32NextW(snap, &mut pe) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
        }
        map
    }

    pub fn has_visible_console(target_pid: u32) -> bool {
        let parents = parent_map();
        visible_console_owner_pids().iter().any(|&owner| {
            // 实测（Win10 22H2）：可见控制台窗口直接属主就是客户端进程本身；
            // 兼容旧模型：属主是 conhost.exe，其父进程才是客户端。
            owner == target_pid || parents.get(&owner) == Some(&target_pid)
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
