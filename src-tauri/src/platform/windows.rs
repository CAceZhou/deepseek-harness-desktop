use super::Platform;
use std::path::{Path, PathBuf};

pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn node_exe_name(&self) -> &'static str {
        "node.exe"
    }

    fn runtime_base_dir(&self) -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("DSHDesktop")
    }

    fn resource_runtime_dir(&self, resource_dir: &Path) -> PathBuf {
        resource_dir.join("runtime").join(self.runtime_triplet())
    }

    fn runtime_triplet(&self) -> &'static str {
        "windows-x64"
    }

    fn kill_process_tree(&self, pid: u32) {
        // /T 杀进程树，/F 强制；失败忽略（进程可能已退出）
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }

    fn configure_child_command(&self, cmd: &mut tokio::process::Command) {
        // tokio 的 Command 在 Windows 上自带 creation_flags
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    fn system_dark_mode(&self) -> bool {
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
            .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
            .map(|v| v == 0)
            .unwrap_or(false)
    }

    fn play_sound_file(&self, path: &Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Media::Audio::PlaySoundW;
        const SND_ASYNC: u32 = 0x0001;
        const SND_FILENAME: u32 = 0x0002_0000;
        const SND_NOSTOP: u32 = 0x0010; // 不打断正在播放的上一声音效（连播排队交给系统混音）
        if !path.is_file() {
            return Err(format!("音效文件不存在: {}", path.display()));
        }
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: wide 以 NUL 结尾且本调用期间存活；hmod 传 NULL（文件模式不需要模块句柄）
        let ok = unsafe {
            PlaySoundW(wide.as_ptr(), std::ptr::null_mut(), SND_FILENAME | SND_ASYNC | SND_NOSTOP)
        };
        if ok == 0 {
            Err("PlaySoundW 播放失败".into())
        } else {
            Ok(())
        }
    }
}
