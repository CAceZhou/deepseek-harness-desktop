//! 插件管理：dsh plugin 官方入口的壳侧封装（npm/cordis 插件）。
//! 装/卸/更新走 `dsh plugin --profile web <pnpm args>`（对账逻辑归上游）；
//! 清单读 profiles/web/package.json（dependencies + dsh.profile.bundles）。
//! pnpm 由壳内置（pnpm.cjs/pnpm.cmd，node.exe 同目录），spawn 时 PATH 前置。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRow {
    pub name: String,
    pub version: String,
    pub is_bundle: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    pub pnpm_ready: bool,
    pub pnpm_version: Option<String>,
    pub profile_ready: bool,
}

/// 插件命令的执行环境（node/dsh/pnpm 全部来自壳分发运行时）。
pub struct PluginsHome {
    pub node_exe: PathBuf,
    pub dsh_bin: PathBuf,
    pub home: PathBuf,
    /// 存放 pnpm.cjs/pnpm.cmd 的目录（= node_exe 所在目录）。
    pub pnpm_dir: PathBuf,
    /// 串行锁：同一时刻只允许一个装/卸/更新操作（防并发写 profile）。
    pub busy: Mutex<()>,
}

impl PluginsHome {
    pub fn new(node_exe: PathBuf, dsh_bin: PathBuf, home: PathBuf) -> Self {
        let pnpm_dir = node_exe
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        Self { node_exe, dsh_bin, home, pnpm_dir, busy: Mutex::new(()) }
    }
    pub fn profile_dir(&self) -> PathBuf {
        crate::upstream::join_segments(&self.home, crate::upstream::PROFILE_DIR_SEGMENTS)
    }
    pub fn manifest_path(&self) -> PathBuf {
        self.profile_dir().join(crate::upstream::PROFILE_MANIFEST_FILE)
    }
    /// 内置 pnpm 入口脚本（fetch-runtime 产出：<pnpm_dir>/pnpm/bin/pnpm.cjs，
    /// 依赖同包 dist/，不能摊平到根目录）。
    pub fn pnpm_js(&self) -> PathBuf {
        self.pnpm_dir
            .join("pnpm")
            .join("bin")
            .join(crate::upstream::PNPM_JS_FILE)
    }
}

fn read_manifest(path: &Path) -> Result<Option<serde_json::Value>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)
        .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes); // BOM 容忍（同 mcp.rs）
    serde_json::from_slice(bytes)
        .map(Some)
        .map_err(|e| format!("解析 {} 失败：{e}（可手工删除该文件让 dsh 重建）", path.display()))
}

pub fn list_plugins_impl(home: &PluginsHome) -> Result<Vec<PluginRow>, String> {
    let Some(m) = read_manifest(&home.manifest_path())? else {
        return Ok(vec![]);
    };
    let deps = m
        .get(crate::upstream::MANIFEST_DEPENDENCIES_KEY)
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let bundles: HashSet<String> = m
        .pointer(crate::upstream::MANIFEST_BUNDLES_POINTER)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mut rows: Vec<PluginRow> = deps
        .into_iter()
        .map(|(name, v)| PluginRow {
            version: v.as_str().unwrap_or("").to_string(),
            is_bundle: bundles.contains(&name),
            name,
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub fn get_plugin_status_impl(home: &PluginsHome) -> Result<PluginStatus, String> {
    let pnpm_js = home.pnpm_js();
    let pnpm_cmd = home.pnpm_dir.join(crate::upstream::PNPM_CMD_FILE);
    let pnpm_ready = pnpm_js.is_file() && pnpm_cmd.is_file();
    let pnpm_version = if pnpm_ready {
        let mut cmd = tokio::process::Command::new(&home.node_exe);
        cmd.arg(&pnpm_js).arg("--version");
        crate::platform::current().configure_child_command(&mut cmd);
        match tauri::async_runtime::block_on(cmd.output()) {
            Ok(o) if o.status.success() => {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            }
            _ => None,
        }
    } else {
        None
    };
    Ok(PluginStatus {
        pnpm_ready,
        pnpm_version,
        profile_ready: home.manifest_path().is_file(),
    })
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOpResult {
    pub exit_code: i32,
    pub output: String, // stdout+stderr 合并，截断
}

const OP_OUTPUT_CAP: usize = 200 * 1024;

fn validate_spec(spec: &str) -> Result<(), String> {
    let s = spec.trim();
    if s.is_empty() {
        return Err("包名不能为空".into());
    }
    if s.len() > 200 {
        return Err("包名过长（≤200 字符）".into());
    }
    if s.starts_with('-') {
        return Err("包名不能以 - 开头（防参数注入）".into());
    }
    Ok(())
}

/// 跑 `node bin.js plugin --profile web <args...>`：DSH_HOME 注入 + PATH 前置
/// pnpm 目录（dsh 内部 spawnSync("pnpm") 经 PATH+PATHEXT 解析到 pnpm.cmd）。
/// 无 shell（杜绝注入）；CREATE_NO_WINDOW（防闪控制台，同 process.rs）；
/// 子进程挂全局 Job Object（壳被杀时连带回收，防孤儿锁 profile）。
pub fn run_plugin_op(home: &PluginsHome, args: &[&str]) -> Result<PluginOpResult, String> {
    if !home.node_exe.is_file() {
        return Err(format!("运行时缺失：{}", home.node_exe.display()));
    }
    if !home.dsh_bin.is_file() {
        return Err(format!("运行时缺失：{}", home.dsh_bin.display()));
    }
    let pnpm_js = home.pnpm_js();
    let pnpm_cmd = home.pnpm_dir.join(crate::upstream::PNPM_CMD_FILE);
    if !pnpm_js.is_file() || !pnpm_cmd.is_file() {
        return Err("内置 pnpm 缺失（pnpm.cmd / pnpm\\bin\\pnpm.cjs）——请重装 DSHDesktop 或重跑 fetch-runtime.ps1".into());
    }
    let mut full_path = home.pnpm_dir.clone().into_os_string();
    full_path.push(";");
    full_path.push(std::env::var_os("PATH").unwrap_or_default());
    let mut cmd = tokio::process::Command::new(&home.node_exe);
    cmd.arg(&home.dsh_bin)
        .arg(crate::upstream::DSH_PLUGIN_SUBCOMMAND)
        .arg(crate::upstream::DSH_PLUGIN_PROFILE_FLAG)
        .arg(crate::upstream::DSH_WEB_PROFILE_NAME)
        .args(args)
        .env("DSH_HOME", &home.home)
        .env("PATH", &full_path);
    // wait_with_output 只读 pipe 出来的句柄；tokio spawn 默认继承父进程
    // stdio，不显式 pipe 则 output 恒为空（“看下方输出”永远没内容）。
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::platform::current().configure_child_command(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("启动 dsh plugin 失败：{e}"))?;
    if let Some(pid) = child.id() {
        crate::platform::current().register_child(pid);
    }
    let out = tauri::async_runtime::block_on(child.wait_with_output())
        .map_err(|e| format!("等待 dsh plugin 结束失败：{e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if text.len() > OP_OUTPUT_CAP {
        text.truncate(OP_OUTPUT_CAP);
    }
    Ok(PluginOpResult { exit_code: out.status.code().unwrap_or(-1), output: text })
}

/// 持锁执行（命令与单测共用）：锁被占时直接报"进行中"。
fn run_op_guarded(home: &PluginsHome, args: &[&str]) -> Result<PluginOpResult, String> {
    let _guard = home
        .busy
        .try_lock()
        .map_err(|_| "已有插件操作在进行中，请等它完成".to_string())?;
    run_plugin_op(home, args)
}

pub fn install_plugin_impl(home: &PluginsHome, spec: &str) -> Result<PluginOpResult, String> {
    validate_spec(spec)?;
    run_op_guarded(home, &["add", spec.trim()])
}

pub fn uninstall_plugin_impl(home: &PluginsHome, name: &str) -> Result<PluginOpResult, String> {
    validate_spec(name)?;
    run_op_guarded(home, &["remove", name.trim()])
}

pub fn update_plugins_impl(home: &PluginsHome) -> Result<PluginOpResult, String> {
    run_op_guarded(home, &["update"])
}

#[tauri::command]
pub fn get_plugin_status(state: tauri::State<PluginsHome>) -> Result<PluginStatus, String> {
    get_plugin_status_impl(state.inner())
}

#[tauri::command]
pub fn list_plugins(state: tauri::State<PluginsHome>) -> Result<Vec<PluginRow>, String> {
    list_plugins_impl(state.inner())
}

#[tauri::command]
pub fn install_plugin(
    state: tauri::State<PluginsHome>,
    spec: String,
) -> Result<PluginOpResult, String> {
    install_plugin_impl(state.inner(), &spec)
}

#[tauri::command]
pub fn uninstall_plugin(
    state: tauri::State<PluginsHome>,
    name: String,
) -> Result<PluginOpResult, String> {
    uninstall_plugin_impl(state.inner(), &name)
}

#[tauri::command]
pub fn update_plugins(state: tauri::State<PluginsHome>) -> Result<PluginOpResult, String> {
    update_plugins_impl(state.inner())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SearchResult {
    pub name: String,
    pub version: String,
    pub description: String,
    pub installed: bool,
}

fn parse_search_response(text: &str, installed: &HashSet<String>) -> Vec<SearchResult> {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
    v.get("objects")
        .and_then(|o| o.as_array())
        .map(|objs| {
            objs.iter()
                .filter_map(|o| o.get("package"))
                .filter_map(|p| {
                    let name = p.get("name")?.as_str()?.to_string();
                    Some(SearchResult {
                        name: name.clone(),
                        version: p
                            .get("version")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        description: p
                            .get("description")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        installed: installed.contains(&name),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn search_plugins(
    state: tauri::State<'_, PluginsHome>,
    query: String,
) -> Result<Vec<SearchResult>, String> {
    let q = query.trim();
    if q.is_empty() || q.chars().count() < 2 {
        return Ok(vec![]);
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("DSHDesktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败：{e}"))?;
    let url = reqwest::Url::parse_with_params(
        "https://registry.npmjs.org/-/v1/search",
        &[("text", q), ("size", "20")],
    )
    .map_err(|e| format!("构造搜索 URL 失败：{e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("npm registry 搜索失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("npm registry 返回 {}", resp.status()));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败：{e}"))?;
    let installed: HashSet<String> = list_plugins_impl(state.inner())?
        .into_iter()
        .map(|r| r.name)
        .collect();
    Ok(parse_search_response(&text, &installed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn system_node() -> PathBuf {
        let out = std::process::Command::new("where").arg("node").output().unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        PathBuf::from(stdout.lines().next().expect("node not found on PATH").trim())
    }

    fn test_home(tag: &str) -> (PluginsHome, PathBuf) {
        let work = std::env::temp_dir().join(format!("dshd-plugins-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let home = PluginsHome {
            node_exe: system_node(),
            dsh_bin: work.join("bin.js"),
            home: work.join("home"),
            pnpm_dir: work.join("bin"),
            busy: Mutex::new(()),
        };
        (home, work)
    }

    #[test]
    fn list_missing_manifest_is_empty() {
        let (home, work) = test_home("list-empty");
        assert!(list_plugins_impl(&home).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn list_reads_deps_and_marks_bundles() {
        let (home, work) = test_home("list-bundles");
        std::fs::create_dir_all(home.profile_dir()).unwrap();
        std::fs::write(
            home.manifest_path(),
            r#"{
              "dependencies": { "@deepseek-ai/dsh-mcp-client": "^0.1.0", "plain-lib": "1.2.3" },
              "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-mcp-client"] } }
            }"#,
        )
        .unwrap();
        let rows = list_plugins_impl(&home).unwrap();
        assert_eq!(rows.len(), 2);
        let mcp = rows.iter().find(|r| r.name == "@deepseek-ai/dsh-mcp-client").unwrap();
        assert!(mcp.is_bundle);
        assert_eq!(mcp.version, "^0.1.0");
        assert!(!rows.iter().find(|r| r.name == "plain-lib").unwrap().is_bundle);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn list_tolerates_bom_and_garbage_version() {
        let (home, work) = test_home("list-bom");
        let dir = home.profile_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(br#"{"dependencies":{"a":{"x":1}}}"#);
        std::fs::write(home.manifest_path(), bytes).unwrap();
        let rows = list_plugins_impl(&home).unwrap();
        assert_eq!(rows[0].version, "");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn list_corrupt_manifest_errors() {
        let (home, work) = test_home("list-corrupt");
        std::fs::create_dir_all(home.profile_dir()).unwrap();
        std::fs::write(home.manifest_path(), "not json").unwrap();
        assert!(list_plugins_impl(&home).is_err());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn status_detects_pnpm_and_profile() {
        let (home, work) = test_home("status");
        std::fs::create_dir_all(home.pnpm_dir.join("pnpm").join("bin")).unwrap();
        std::fs::write(home.pnpm_dir.join("pnpm").join("bin").join("pnpm.cjs"), "console.log('9.15.0')").unwrap();
        std::fs::write(home.pnpm_dir.join("pnpm.cmd"), "@echo off").unwrap();
        let s = get_plugin_status_impl(&home).unwrap();
        assert!(s.pnpm_ready && !s.profile_ready);
        assert_eq!(s.pnpm_version.as_deref(), Some("9.15.0"));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn validate_rejects_empty_flaglike_and_long() {
        assert!(validate_spec("").is_err());
        assert!(validate_spec("  ").is_err());
        assert!(validate_spec("-g").is_err());
        assert!(validate_spec(&"a".repeat(201)).is_err());
        assert!(validate_spec("  @deepseek-ai/dsh-mcp-client  ").is_ok());
    }

    #[test]
    fn run_missing_pnpm_errors() {
        let (home, work) = test_home("run-missing-pnpm");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(&home.dsh_bin, "dummy").unwrap(); // node/dsh 齐备，缺的只有 pnpm
        std::fs::create_dir_all(&home.pnpm_dir).unwrap();
        assert!(run_plugin_op(&home, &["add", "x"]).is_err());
        assert!(run_plugin_op(&home, &["add", "x"]).unwrap_err().contains("pnpm"));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn busy_lock_serializes() {
        let (home, work) = test_home("busy");
        let _g = home.busy.try_lock().unwrap(); // 模拟已有操作在跑
        assert!(run_op_guarded(&home, &["add", "x"]).is_err());
        assert!(run_op_guarded(&home, &["add", "x"]).unwrap_err().contains("进行中"));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn search_parses_registry_response_and_marks_installed() {
        let json = r#"{"objects":[
            {"package":{"name":"dsh-mcp-client","version":"1.0.0","description":"MCP for dsh"}},
            {"package":{"name":"other","version":"2.0.0"}}
        ],"total":2}"#;
        let mut installed = HashSet::new();
        installed.insert("other".to_string());
        let r = parse_search_response(json, &installed);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].name, "dsh-mcp-client");
        assert!(!r[0].installed && r[0].description == "MCP for dsh");
        assert!(r[1].installed);
        assert_eq!(r[1].description, "");
    }

    #[test]
    fn run_captures_child_output() {
        // “看下方输出”依赖捕获子进程 stdout/stderr；tokio spawn 默认继承
        // 父进程 stdio，不显式 pipe 则 output 恒为空。
        let (home, work) = test_home("capture");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            &home.dsh_bin,
            r#"console.log("out-marker"); console.error("err-marker"); process.exit(3);"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.pnpm_dir.join("pnpm").join("bin")).unwrap();
        std::fs::write(home.pnpm_dir.join("pnpm").join("bin").join("pnpm.cjs"), "").unwrap();
        std::fs::write(home.pnpm_dir.join("pnpm.cmd"), "").unwrap();
        let r = run_plugin_op(&home, &["add", "x"]).unwrap();
        assert_eq!(r.exit_code, 3);
        assert!(r.output.contains("out-marker"), "stdout 未被捕获：{:?}", r.output);
        assert!(r.output.contains("err-marker"), "stderr 未被捕获：{:?}", r.output);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn ipc_payloads_serialize_camel_case() {
        // 前端（Plugins.svelte）按 camelCase 读键；snake_case 会让 exitCode 恒为
        // undefined——成功被误判为失败、pnpm 状态恒显示"缺失"。
        let v = serde_json::to_value(PluginOpResult { exit_code: 0, output: "x".into() }).unwrap();
        assert!(v.get("exitCode").is_some(), "前端读 exitCode，实际键：{v}");
        let v = serde_json::to_value(PluginStatus {
            pnpm_ready: true,
            pnpm_version: Some("9.15.0".into()),
            profile_ready: true,
        })
        .unwrap();
        assert!(v.get("pnpmReady").is_some(), "前端读 pnpmReady，实际键：{v}");
        assert!(v.get("pnpmVersion").is_some());
        assert!(v.get("profileReady").is_some());
        let v = serde_json::to_value(PluginRow {
            name: "a".into(),
            version: "1".into(),
            is_bundle: true,
        })
        .unwrap();
        assert!(v.get("isBundle").is_some(), "前端读 isBundle，实际键：{v}");
    }

    #[test]
    fn search_parses_garbage_as_empty() {
        assert!(parse_search_response("not json", &HashSet::new()).is_empty());
    }
}
