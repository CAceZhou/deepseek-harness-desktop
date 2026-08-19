//! 插件命令端到端：真实 dsh bin.js 的 plugin 模式 + 假 pnpm.cmd。
//! 断言：profile 初始化、pnpm 收到正确参数且 cwd=profile 目录、退出码透传、
//! DSH_HOME/PATH 注入生效。无运行时（runtime/windows-x64 缺失）则 skip。

use dshdesktop_lib::plugins::{
    install_plugin_impl, uninstall_plugin_impl, update_plugins_impl, PluginsHome,
};
use dshdesktop_lib::upstream::dsh_bin;
use std::path::PathBuf;
use std::sync::Mutex;

fn runtime_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("DSHDESKTOP_RUNTIME_DIR") {
        return Some(PathBuf::from(d));
    }
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("runtime")
        .join("windows-x64");
    p.is_dir().then_some(p)
}

fn system_node() -> PathBuf {
    let out = std::process::Command::new("where").arg("node").output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    PathBuf::from(stdout.lines().next().expect("node not found on PATH").trim())
}

/// 假 pnpm：把 cwd 与参数追加写进 pnpm.log（%~dp0 = pnpm.cmd 所在目录，
/// 无需环境变量注入）；存在 exit-1 控制文件时退出码 1。
/// `echo(` 语法避免 `echo %CD% >>` 在行尾追加空格。
const FAKE_PNPM_CMD: &str = "@echo off\r\necho(%CD%>>\"%~dp0pnpm.log\"\r\necho(%*>>\"%~dp0pnpm.log\"\r\nif exist \"%~dp0exit-1\" exit /b 1\r\nexit /b 0\r\n";

fn test_home(tag: &str) -> (PluginsHome, PathBuf) {
    let work = std::env::temp_dir().join(format!("dshd-plugins-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("bin").join("pnpm").join("bin")).unwrap();
    std::fs::write(work.join("bin").join("pnpm.cmd"), FAKE_PNPM_CMD).unwrap();
    std::fs::write(work.join("bin").join("pnpm").join("bin").join("pnpm.cjs"), "console.log('fake')").unwrap();
    let home = PluginsHome {
        node_exe: system_node(),
        dsh_bin: dsh_bin(&runtime_dir().expect("runtime dir")),
        home: work.join("home"),
        pnpm_dir: work.join("bin"),
        busy: Mutex::new(()),
    };
    (home, work)
}

#[test]
fn install_forwards_args_with_profile_cwd_and_inits_profile() {
    let Some(_rt) = runtime_dir() else {
        eprintln!("skipped: 无真实运行时");
        return;
    };
    let (home, work) = test_home("install");
    let res = install_plugin_impl(&home, "some-pkg").unwrap();
    assert_eq!(res.exit_code, 0, "输出：{}", res.output);
    let log = std::fs::read_to_string(home.pnpm_dir.join("pnpm.log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines[0], home.profile_dir().display().to_string(), "cwd 应为 profile 目录");
    assert_eq!(lines[1], "add some-pkg", "参数应原样透传");
    assert!(home.manifest_path().is_file(), "dsh 应初始化 profile 清单");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn uninstall_and_update_forward_verbatim() {
    let Some(_rt) = runtime_dir() else {
        eprintln!("skipped");
        return;
    };
    let (home, work) = test_home("others");
    assert_eq!(uninstall_plugin_impl(&home, "foo").unwrap().exit_code, 0);
    assert_eq!(update_plugins_impl(&home).unwrap().exit_code, 0);
    let log = std::fs::read_to_string(home.pnpm_dir.join("pnpm.log")).unwrap();
    assert!(log.contains("remove foo"));
    assert!(log.contains("update"));
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn pnpm_failure_exit_code_passthrough() {
    let Some(_rt) = runtime_dir() else {
        eprintln!("skipped");
        return;
    };
    let (home, work) = test_home("fail");
    std::fs::write(home.pnpm_dir.join("exit-1"), "").unwrap();
    let res = install_plugin_impl(&home, "boom").unwrap();
    assert_eq!(res.exit_code, 1);
    let _ = std::fs::remove_dir_all(&work);
}
