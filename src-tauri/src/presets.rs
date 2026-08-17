//! 启动期修复 dsh 随包预设的 Windows 缺口。
//!
//! dsh rc 的 `minimal`（极简模式）预设无条件挂载 PTY 持久 bash
//! （dsh-terminal + dsh-terminal-bash + dsh-tool-bash-persistent），而
//! dsh-subprocess-local 的终端检查器只实现了 linux/darwin——win32 上
//! `createProcessInspector()` 直接抛错，每次 bash 调用都以
//! "terminal inspection is unsupported on platform win32" 失败。
//! standard/code/cordis 预设都有平台分支（win32 换 tool-pwsh），唯独 minimal 漏了。
//!
//! 不能走 profile patch 注入影子预设根：dsh 的 composeProfile 会无条件把
//! agent-presets 行的 roots 重写为 shipped root（用户 patch 层的 roots 被覆盖），
//! shipped root 又在发现顺序上先于用户根（同名 id shipped 优先）。因此只能在
//! dsh 启动前原地改写 shipped 预设文件。签名门控保证上游修复后本补丁自动停手；
//! dsh 自更新还原文件后，下次启动重新打补丁。

use crate::runtime::RuntimePaths;
use std::fs;
use std::path::Path;

/// 补丁标记：写入文件首行注释，幂等判定用。
const MARKER: &str = "dshdesktop: win32 pwsh variant v1";
/// 破损签名：文件引用了 PTY 持久 bash 工具。
const BROKEN_NEEDLE: &str = "dsh-tool-bash-persistent";
/// 上游若引入平台分支（内容出现 win32），视为已自行修复，不再覆盖。
const PLATFORM_NEEDLE: &str = "win32";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOutcome {
    /// 已改写为 Windows 变体。
    Patched,
    ///  marker 已在，幂等跳过。
    AlreadyPatched,
    /// 内容不含破损签名或已含平台分支：上游变了，不动它。
    UpstreamChanged,
    /// 预设文件不存在（fixture 运行时等布局），无事可做。
    Missing,
}

/// win32 下把 shipped minimal 预设改写为 pwsh 变体；返回处理结果供记日志。
/// 其它平台直接返回 UpstreamChanged（不掺和）。任何 IO 失败只记日志不阻断启动。
pub fn patch_minimal_preset(paths: &RuntimePaths) -> PatchOutcome {
    if !cfg!(windows) {
        return PatchOutcome::UpstreamChanged;
    }
    // dsh_bin = <pkg>/lib/bin.js → 预设目录 = <pkg>/config/agent-presets/minimal
    let Some(pkg_dir) = paths.dsh_bin.parent().and_then(|p| p.parent()) else {
        return PatchOutcome::Missing;
    };
    let dir = pkg_dir.join("config").join("agent-presets").join("minimal");
    match patch_dir(&dir) {
        Ok(o) => o,
        Err(_) => PatchOutcome::Missing,
    }
}

fn patch_dir(dir: &Path) -> std::io::Result<PatchOutcome> {
    let composition = dir.join("agent.cordis.yml");
    if !composition.is_file() {
        return Ok(PatchOutcome::Missing);
    }
    let content = fs::read_to_string(&composition)?;
    if content.contains(MARKER) {
        return Ok(PatchOutcome::AlreadyPatched);
    }
    if !content.contains(BROKEN_NEEDLE) || content.contains(PLATFORM_NEEDLE) {
        return Ok(PatchOutcome::UpstreamChanged);
    }
    write_atomic(&composition, WIN32_COMPOSITION)?;
    // 简介里"持久 bash"在 Windows 变体下不再准确，同步改写（缺失则不管）。
    let meta = dir.join("preset.yml");
    if meta.is_file() {
        let m = fs::read_to_string(&meta)?;
        if !m.contains(MARKER) {
            write_atomic(&meta, WIN32_PRESET_YML)?;
        }
    }
    Ok(PatchOutcome::Patched)
}

/// tmp+rename 原子写：dsh 侧可能正读着（预设发现是每次调用重扫的）。
fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("yml.dshdesktop-tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// minimal 预设的 Windows 变体：PTY 持久 bash 换成 tool-pwsh。
/// tool-pwsh 注册进 host 的 tools 注册表、自身不提供服务，按 standard 预设的
/// 注释无需 isolate realm；其执行器 pwsh-sandbox 在 base patch 里仅 win32 启用。
/// persona 保持 complete 固定提示词风格，但补上平台与工作目录事实——原版
/// includeRuntimeContext:false 又不给 cwd，模型只能靠猜（在 Windows 上会
/// 试探 "/" / "C:\\"，撞上 fs-local 列目录遇 ACL 拒绝项即整列失败的上游行为）。
/// {{cwd}} 由 dsh-agent-loop 注册的 prompt 变量插值（standard 预设同款用法）。
const WIN32_COMPOSITION: &str = r#"# dshdesktop: win32 pwsh variant v1
#
# Upstream rc mounts a PTY-backed persistent bash here, whose terminal
# inspector is unimplemented on win32 (createProcessInspector throws), so
# every bash call fails. This variant swaps in tool-pwsh: the host-plane
# pwsh-sandbox executor uses pipe spawns and works on Windows. Semantic
# difference: each call is a fresh pwsh process, no cross-call shell state.
# This patch is signature-gated; once upstream handles win32 it stops applying.

- id: persona
  name: '@deepseek-ai/dsh-persona'
  config:
    text: >-
      You are a helpful software engineer assistant. You are running on
      Windows: the shell tool executes Windows PowerShell, not bash — use
      PowerShell syntax and native Windows paths (C:\...), and read
      environment variables with $env:NAME. Your working directory is
      {{cwd}}; prefer absolute paths under it.
    complete: true
    includeRuntimeContext: false

- id: tool-pwsh
  name: '@deepseek-ai/dsh-tool-pwsh'

# The bare local filesystem shadows the host's sandboxed provider only for this
# preset. The editor shares that realm and requires absolute paths.
- id: filesystem
  name: cordis:group
  group: true
  isolate:
    fs: true
  config:
    - id: fs-local
      name: '@deepseek-ai/dsh-fs-local'
      config:
        cwd: !!js process.env.DSH_CWD ?? process.cwd()

    - id: str-replace-editor
      name: '@deepseek-ai/dsh-tool-str-replace-editor'
      config:
        maxOutputChars: 16000
"#;

const WIN32_PRESET_YML: &str = "name: 极简模式\ndescription: 双工具编码 Agent（Windows 适配：以 PowerShell 替代持久 bash）。\norder: 3\n";

#[cfg(test)]
mod tests {
    use super::*;

    /// 上游 rc.6 的 minimal 预设（节选关键行，签名与平台判定只看特征串）。
    const UPSTREAM_COMPOSITION: &str = r#"# The `minimal` agent preset
- id: persona
  name: '@deepseek-ai/dsh-persona'
- id: persistent-shell
  name: cordis:group
  group: true
  config:
    - id: persistent-bash
      name: '@deepseek-ai/dsh-tool-bash-persistent'
- id: filesystem
  name: cordis:group
"#;
    const UPSTREAM_PRESET_YML: &str =
        "name: 极简模式\ndescription: 仅提供持久 bash 与 str_replace_editor 的双工具编码 Agent。\norder: 3\n";

    fn make_preset() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("agent.cordis.yml"), UPSTREAM_COMPOSITION).unwrap();
        fs::write(dir.path().join("preset.yml"), UPSTREAM_PRESET_YML).unwrap();
        dir
    }

    #[test]
    fn patches_broken_upstream() {
        let dir = make_preset();
        assert_eq!(patch_dir(dir.path()).unwrap(), PatchOutcome::Patched);
        let c = fs::read_to_string(dir.path().join("agent.cordis.yml")).unwrap();
        assert!(c.contains(MARKER));
        assert!(c.contains("dsh-tool-pwsh"));
        assert!(!c.contains(BROKEN_NEEDLE), "PTY bash 行应被移除");
        assert!(c.contains("{{cwd}}"), "persona 应告知工作目录");
        // 文件系统组原样保留
        assert!(c.contains("dsh-fs-local") && c.contains("str-replace-editor"));
        let m = fs::read_to_string(dir.path().join("preset.yml")).unwrap();
        assert!(m.contains("PowerShell"), "简介应同步更新");
        assert!(m.contains("极简模式"), "显示名不变");
    }

    #[test]
    fn idempotent_when_marker_present() {
        let dir = make_preset();
        assert_eq!(patch_dir(dir.path()).unwrap(), PatchOutcome::Patched);
        assert_eq!(patch_dir(dir.path()).unwrap(), PatchOutcome::AlreadyPatched);
    }

    #[test]
    fn skips_when_upstream_adds_platform_branch() {
        let dir = tempfile::tempdir().unwrap();
        // 假想上游修复：仍含持久 bash 但加了 win32 条件
        let fixed = UPSTREAM_COMPOSITION
            .replace("name: '@deepseek-ai/dsh-tool-bash-persistent'", "name: '@deepseek-ai/dsh-tool-bash-persistent'\n      disabled: !!js process.platform === 'win32'");
        fs::write(dir.path().join("agent.cordis.yml"), fixed).unwrap();
        assert_eq!(patch_dir(dir.path()).unwrap(), PatchOutcome::UpstreamChanged);
        let c = fs::read_to_string(dir.path().join("agent.cordis.yml")).unwrap();
        assert!(c.contains("win32"), "上游内容不应被覆盖");
    }

    #[test]
    fn skips_when_shape_changed() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("agent.cordis.yml"), "# completely new minimal\n").unwrap();
        assert_eq!(patch_dir(dir.path()).unwrap(), PatchOutcome::UpstreamChanged);
    }

    #[test]
    fn missing_dir_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(patch_dir(dir.path()).unwrap(), PatchOutcome::Missing);
    }

    #[test]
    fn missing_preset_yml_still_patches_composition() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("agent.cordis.yml"), UPSTREAM_COMPOSITION).unwrap();
        assert_eq!(patch_dir(dir.path()).unwrap(), PatchOutcome::Patched);
    }

    #[test]
    fn no_tmp_residue() {
        let dir = make_preset();
        patch_dir(dir.path()).unwrap();
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 2, "不应残留临时文件");
    }
}
