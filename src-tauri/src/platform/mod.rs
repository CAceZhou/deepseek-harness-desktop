use std::path::{Path, PathBuf};

/// 平台抽象层：所有平台相关的路径、可执行文件名、进程树回收都收敛在这里。
/// 后期支持 macOS/Linux 时，新增 platform/macos.rs / platform/linux.rs 实现本 trait，
/// 并替换下面的 compile_error! 占位。
pub trait Platform: Send + Sync {
    fn node_exe_name(&self) -> &'static str;
    /// 应用数据根目录（Windows: %LOCALAPPDATA%\DSHDesktop）
    fn runtime_base_dir(&self) -> PathBuf;
    /// 安装包资源目录中内嵌运行时所在路径：<resource_dir>/runtime/<triplet>
    fn resource_runtime_dir(&self, resource_dir: &Path) -> PathBuf;
    fn runtime_triplet(&self) -> &'static str;
    /// 回收整个进程树（dsh 可能派生 python 等子进程）
    fn kill_process_tree(&self, pid: u32);
    /// 子进程创建前的平台化配置（Windows 设 CREATE_NO_WINDOW 隐藏控制台窗口）
    fn configure_child_command(&self, _cmd: &mut tokio::process::Command) {}
    /// 系统是否处于深色模式（dsh 主题为 system 时用来解析）
    fn system_dark_mode(&self) -> bool;
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::WindowsPlatform;

#[cfg(target_os = "macos")]
compile_error!("macOS platform implementation pending: add platform/macos.rs implementing Platform");
#[cfg(target_os = "linux")]
compile_error!("Linux platform implementation pending: add platform/linux.rs implementing Platform");

pub fn current() -> Box<dyn Platform> {
    #[cfg(windows)]
    {
        Box::new(WindowsPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_platform_basics() {
        let p = current();
        assert_eq!(p.node_exe_name(), "node.exe");
        assert_eq!(p.runtime_triplet(), "windows-x64");
        assert!(p.runtime_base_dir().ends_with("DSHDesktop"));
        let r = p.resource_runtime_dir(Path::new("C:\\res"));
        assert_eq!(r, PathBuf::from("C:\\res").join("runtime").join("windows-x64"));
    }
}
