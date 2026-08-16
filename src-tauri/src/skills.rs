//! 技能管理：列出/启停/删除 <dsh-home>/skills 的技能，并从外部 agent 的技能目录导入。
//!
//! dsh 的技能发现（dsh-skill-filesystem）只认各根目录的直属条目，没有原生禁用概念；
//! 壳侧把停用技能的目录移到旁路 skills-disabled/（不是任何发现根），dsh 的 watcher
//! 观察到根目录变化后热刷新 catalog，启停即时生效、无需重启。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

/// dsh 实际使用的 home（DSH_HOME），由 setup 注入（runtime_base_dir/dsh-home）
pub struct SkillsHome(pub PathBuf);

const DISABLED_DIR: &str = "skills-disabled";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillRow {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SourceSkill {
    pub name: String,
    pub description: String,
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SourceInfo {
    pub id: String,
    pub label: String,
    pub exists: bool,
    pub skills: Vec<SourceSkill>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportItem {
    pub name: String,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImportResult {
    pub name: String,
    pub status: String, // "imported" | "skipped" | "error"
    pub error: Option<String>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// 取 SKILL.md frontmatter 的 description（只认首个 --- 对之间的单行键，允许引号；
/// 多行 folded YAML 不支持——四个来源的技能都是单行）
fn parse_description(content: &str) -> String {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return String::new();
    }
    for line in lines {
        let t = line.trim();
        if t == "---" {
            break;
        }
        if let Some(v) = t.strip_prefix("description:") {
            return unquote(v.trim());
        }
    }
    String::new()
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

/// 扫一个技能根目录：只收"含 SKILL.md 的目录"（与 dsh discoverRoot 的跳过语义一致）；
/// name 取目录名（导入/启停/删除都按目录名操作；frontmatter 名实践中与目录名一致）
fn scan_skills_dir(dir: &Path, enabled: bool) -> Vec<SkillRow> {
    let mut rows = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return rows;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(content) = fs::read_to_string(path.join("SKILL.md")) else {
            continue;
        };
        rows.push(SkillRow {
            name: entry.file_name().to_string_lossy().into_owned(),
            description: parse_description(&content),
            enabled,
        });
    }
    rows
}

fn list_skills_root(home: &Path) -> Vec<SkillRow> {
    let mut rows = scan_skills_dir(&home.join("skills"), true);
    rows.extend(scan_skills_dir(&home.join(DISABLED_DIR), false));
    rows.sort_by(|a, b| b.enabled.cmp(&a.enabled).then_with(|| a.name.cmp(&b.name)));
    rows
}

fn set_enabled(home: &Path, name: &str, enabled: bool) -> Result<(), String> {
    if !valid_name(name) {
        return Err(crate::i18n::pick(format!("非法技能名: {name}"), format!("Invalid skill name: {name}")));
    }
    let (from, to) = if enabled {
        (home.join(DISABLED_DIR).join(name), home.join("skills").join(name))
    } else {
        (home.join("skills").join(name), home.join(DISABLED_DIR).join(name))
    };
    if !from.is_dir() {
        return Err(crate::i18n::pick(format!("技能不存在: {name}"), format!("Skill not found: {name}")));
    }
    if to.exists() {
        return Err(crate::i18n::pick(
            format!("另一侧已存在同名技能: {name}"),
            format!("A skill with the same name already exists on the other side: {name}"),
        ));
    }
    fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&from, &to).map_err(|e| e.to_string())
}

fn delete_skill_dir(home: &Path, name: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err(crate::i18n::pick(format!("非法技能名: {name}"), format!("Invalid skill name: {name}")));
    }
    for dir in [home.join("skills").join(name), home.join(DISABLED_DIR).join(name)] {
        if dir.is_dir() {
            return fs::remove_dir_all(&dir).map_err(|e| e.to_string());
        }
    }
    Err(crate::i18n::pick(format!("技能不存在: {name}"), format!("Skill not found: {name}")))
}

/// 四个导入源（用户级目录；不存在时在弹窗里灰显）
fn sources() -> Vec<(String, String, PathBuf)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        ("codex", "Codex", ".codex/skills"),
        ("claude", "Claude Code", ".claude/skills"),
        ("opencode", "OpenCode", ".config/opencode/skills"),
    ]
    .into_iter()
    .map(|(id, label, rel)| (id.to_string(), label.to_string(), home.join(rel)))
    .collect()
}

/// 启动时自动导入 ~/.dsh/skills（独立 dsh 的默认技能目录——壳隔离了 DSH_HOME，
/// 但用户心智里"我就是 dsh"，不该手动导入）。每次启动都扫描新技能；
/// marker 文件 `.skills-seeded` 记录已见过的名字：再次扫描不重复导入，
/// 用户从壳里删掉的技能也不会复活。源目录不存在 = 无操作。
pub fn seed_from_default_dsh_home(home: &Path) {
    let Some(user_home) = dirs::home_dir() else {
        return;
    };
    if let Err(e) = seed_auto_import(&user_home.join(".dsh/skills"), home) {
        // events.log 在 home 的父目录（runtime_base_dir）
        if let Some(base) = home.parent() {
            crate::append_debug_line(&base.join("events.log"), &format!("skills seed failed: {e}"));
        }
    }
}

fn seed_auto_import(source_dir: &Path, home: &Path) -> Result<(), String> {
    use std::collections::HashSet;
    let marker = home.join(".skills-seeded");
    let mut seen: HashSet<String> = fs::read_to_string(&marker)
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();

    if source_dir.is_dir() {
        let target_root = home.join("skills");
        for row in scan_skills_dir(source_dir, true) {
            if !seen.insert(row.name.clone()) {
                continue; // 见过（含被用户删除的）——不重复导入、不复活
            }
            // 壳里已有（含停用态）：不动内容，只记入 marker
            let exists = target_root.join(&row.name).exists()
                || home.join(DISABLED_DIR).join(&row.name).exists();
            if !exists && valid_name(&row.name) {
                fs::create_dir_all(&target_root).map_err(|e| e.to_string())?;
                fs_extra::dir::copy(
                    source_dir.join(&row.name),
                    &target_root,
                    &fs_extra::dir::CopyOptions::new(),
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    let mut names: Vec<_> = seen.into_iter().collect();
    names.sort();
    fs::create_dir_all(home).map_err(|e| e.to_string())?;
    fs::write(&marker, names.join("\n") + "\n").map_err(|e| e.to_string())
}

fn source_info((id, label, dir): &(String, String, PathBuf), home: &Path) -> SourceInfo {
    let exists = dir.is_dir();
    let skills = if exists {
        scan_skills_dir(dir, true)
            .into_iter()
            .map(|row| {
                let conflict = home.join("skills").join(&row.name).exists()
                    || home.join(DISABLED_DIR).join(&row.name).exists();
                SourceSkill { name: row.name, description: row.description, conflict }
            })
            .collect()
    } else {
        Vec::new()
    };
    SourceInfo { id: id.clone(), label: label.clone(), exists, skills }
}

fn import_from(source_dir: &Path, home: &Path, items: &[ImportItem]) -> Vec<ImportResult> {
    items.iter().map(|item| import_one(source_dir, home, item)).collect()
}

fn import_one(source_dir: &Path, home: &Path, item: &ImportItem) -> ImportResult {
    let name = item.name.clone();
    let err = |e: &str| ImportResult { name: name.clone(), status: "error".into(), error: Some(e.to_string()) };
    if !valid_name(&item.name) {
        return err(&crate::i18n::pick("非法技能名", "Invalid skill name"));
    }
    let src = source_dir.join(&item.name);
    if !src.join("SKILL.md").is_file() {
        return err(&crate::i18n::pick("源目录中不存在该技能", "Skill not found in the source directory"));
    }
    let target_root = home.join("skills");
    let conflict =
        target_root.join(&item.name).exists() || home.join(DISABLED_DIR).join(&item.name).exists();
    if conflict && !item.overwrite {
        return ImportResult { name, status: "skipped".into(), error: None };
    }
    if conflict {
        // 覆盖 = 清掉两侧旧副本再复制（禁用目录里的也算冲突，防止日后启用出双份）
        let _ = fs::remove_dir_all(target_root.join(&item.name));
        let _ = fs::remove_dir_all(home.join(DISABLED_DIR).join(&item.name));
    }
    if let Err(e) = fs::create_dir_all(&target_root) {
        return err(&e.to_string());
    }
    match fs_extra::dir::copy(&src, &target_root, &fs_extra::dir::CopyOptions::new()) {
        Ok(_) => ImportResult { name, status: "imported".into(), error: None },
        Err(e) => err(&e.to_string()),
    }
}

#[tauri::command]
pub fn list_skills(state: State<SkillsHome>) -> Vec<SkillRow> {
    list_skills_root(&state.0)
}

#[tauri::command]
pub fn list_import_sources(state: State<SkillsHome>) -> Vec<SourceInfo> {
    sources().iter().map(|s| source_info(s, &state.0)).collect()
}

#[tauri::command]
pub fn import_skills(
    state: State<SkillsHome>,
    source_id: String,
    items: Vec<ImportItem>,
) -> Vec<ImportResult> {
    let Some((.., dir)) = sources().into_iter().find(|(id, ..)| *id == source_id) else {
        return items
            .iter()
            .map(|i| ImportResult {
                name: i.name.clone(),
                status: "error".into(),
                error: Some(crate::i18n::pick("未知导入源", "Unknown import source")),
            })
            .collect();
    };
    import_from(&dir, &state.0, &items)
}

#[tauri::command]
pub fn set_skill_enabled(state: State<SkillsHome>, name: String, enabled: bool) -> Result<(), String> {
    set_enabled(&state.0, &name, enabled)
}

#[tauri::command]
pub fn delete_skill(state: State<SkillsHome>, name: String) -> Result<(), String> {
    delete_skill_dir(&state.0, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, dir_name: &str, desc: &str) {
        let d = root.join(dir_name);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {dir_name}\ndescription: \"{desc}\"\n---\n\nbody\n"),
        )
        .unwrap();
    }

    // ---- parse_description ----

    #[test]
    fn parse_quoted_desc() {
        let d = parse_description("---\nname: a\ndescription: \"hello world\"\n---\nbody");
        assert_eq!(d, "hello world");
    }

    #[test]
    fn parse_unquoted_desc_with_colon() {
        let d = parse_description("---\ndescription: Use when x: y\n---\n");
        assert_eq!(d, "Use when x: y");
    }

    #[test]
    fn parse_missing_frontmatter() {
        assert_eq!(parse_description("# no frontmatter\nbody"), "");
        assert_eq!(parse_description(""), "");
    }

    // ---- list ----

    #[test]
    fn list_shows_enabled_and_disabled_sorted() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        write_skill(&home.join("skills"), "b-skill", "on");
        write_skill(&home.join(DISABLED_DIR), "a-skill", "off");
        let rows = list_skills_root(home);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "b-skill"); // 启用在前
        assert!(rows[0].enabled);
        assert_eq!(rows[1].name, "a-skill");
        assert!(!rows[1].enabled);
        assert_eq!(rows[1].description, "off");
    }

    #[test]
    fn list_skips_dir_without_skill_md() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("skills").join("empty-dir")).unwrap();
        assert!(list_skills_root(t.path()).is_empty());
    }

    // ---- set_enabled ----

    #[test]
    fn toggle_moves_dir_forth_and_back() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        write_skill(&home.join("skills"), "x", "d");
        set_enabled(home, "x", false).unwrap();
        assert!(!home.join("skills/x").exists());
        assert!(home.join(format!("{DISABLED_DIR}/x/SKILL.md")).is_file());
        set_enabled(home, "x", true).unwrap();
        assert!(home.join("skills/x/SKILL.md").is_file());
    }

    #[test]
    fn toggle_missing_or_conflict_errors() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        assert!(set_enabled(home, "ghost", false).is_err());
        write_skill(&home.join("skills"), "x", "d");
        write_skill(&home.join(DISABLED_DIR), "x", "d");
        assert!(set_enabled(home, "x", false).is_err()); // 目标已存在
        assert!(set_enabled(home, "..", false).is_err()); // 穿越名
    }

    // ---- delete ----

    #[test]
    fn delete_removes_from_either_dir() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        write_skill(&home.join("skills"), "on", "d");
        write_skill(&home.join(DISABLED_DIR), "off", "d");
        delete_skill_dir(home, "on").unwrap();
        delete_skill_dir(home, "off").unwrap();
        assert!(!home.join("skills/on").exists());
        assert!(!home.join(format!("{DISABLED_DIR}/off")).exists());
        assert!(delete_skill_dir(home, "ghost").is_err());
        assert!(delete_skill_dir(home, "a/b").is_err());
    }

    // ---- import ----

    #[test]
    fn import_copies_recursively() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "deep", "d");
        fs::create_dir_all(src.join("deep/scripts")).unwrap();
        fs::write(src.join("deep/scripts/run.sh"), "echo hi").unwrap();
        let items = vec![ImportItem { name: "deep".into(), overwrite: false }];
        let r = import_from(&src, &home, &items);
        assert_eq!(r[0].status, "imported");
        assert!(home.join("skills/deep/scripts/run.sh").is_file());
    }

    #[test]
    fn import_conflict_skip_and_overwrite() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "x", "new-version");
        write_skill(&home.join("skills"), "x", "old-version");

        let skip = import_from(&src, &home, &[ImportItem { name: "x".into(), overwrite: false }]);
        assert_eq!(skip[0].status, "skipped");
        let body = fs::read_to_string(home.join("skills/x/SKILL.md")).unwrap();
        assert!(body.contains("old-version"));

        let ow = import_from(&src, &home, &[ImportItem { name: "x".into(), overwrite: true }]);
        assert_eq!(ow[0].status, "imported");
        let body = fs::read_to_string(home.join("skills/x/SKILL.md")).unwrap();
        assert!(body.contains("new-version"));
    }

    #[test]
    fn import_overwrite_also_clears_disabled_copy() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "x", "new");
        write_skill(&home.join(DISABLED_DIR), "x", "old"); // 冲突在禁用目录
        let r = import_from(&src, &home, &[ImportItem { name: "x".into(), overwrite: true }]);
        assert_eq!(r[0].status, "imported");
        assert!(home.join("skills/x/SKILL.md").is_file());
        assert!(!home.join(format!("{DISABLED_DIR}/x")).exists());
    }

    #[test]
    fn import_rejects_traversal_and_missing() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        fs::create_dir_all(&src).unwrap();
        let items = vec![
            ImportItem { name: "..".into(), overwrite: false },
            ImportItem { name: "a/b".into(), overwrite: false },
            ImportItem { name: "ghost".into(), overwrite: false },
        ];
        let r = import_from(&src, &home, &items);
        assert!(r.iter().all(|x| x.status == "error"));
        assert!(!home.join("skills").exists() || fs::read_dir(home.join("skills")).unwrap().next().is_none());
    }

    // ---- source_info ----

    #[test]
    fn source_info_marks_conflict_including_disabled() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "dup", "d");
        write_skill(&src, "fresh", "d");
        write_skill(&home.join(DISABLED_DIR), "dup", "d");
        let s = ("codex".to_string(), "Codex".to_string(), src);
        let info = source_info(&s, &home);
        assert!(info.exists);
        assert_eq!(info.skills.len(), 2);
        let dup = info.skills.iter().find(|k| k.name == "dup").unwrap();
        let fresh = info.skills.iter().find(|k| k.name == "fresh").unwrap();
        assert!(dup.conflict);
        assert!(!fresh.conflict);
    }

    #[test]
    fn source_info_missing_dir() {
        let t = tempfile::tempdir().unwrap();
        let s = ("codex".to_string(), "Codex".to_string(), t.path().join("nope"));
        let info = source_info(&s, t.path());
        assert!(!info.exists);
        assert!(info.skills.is_empty());
    }

    #[test]
    fn valid_name_rules() {
        assert!(valid_name("brainstorming"));
        assert!(!valid_name(""));
        assert!(!valid_name("."));
        assert!(!valid_name(".."));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("a\\b"));
    }

    // ---- seed_auto_import ----

    #[test]
    fn seed_imports_all_and_writes_marker() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "a", "1");
        write_skill(&src, "b", "2");
        seed_auto_import(&src, &home).unwrap();
        assert!(home.join("skills/a/SKILL.md").is_file());
        assert!(home.join("skills/b/SKILL.md").is_file());
        let marker = fs::read_to_string(home.join(".skills-seeded")).unwrap();
        assert!(marker.contains("a") && marker.contains("b"));
    }

    #[test]
    fn seed_does_not_resurrect_deleted() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "a", "1");
        seed_auto_import(&src, &home).unwrap();
        delete_skill_dir(&home, "a").unwrap(); // 用户从壳里删掉
        seed_auto_import(&src, &home).unwrap(); // 再扫：不得复活
        assert!(!home.join("skills/a").exists());
    }

    #[test]
    fn seed_picks_up_new_skills_later() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "a", "1");
        seed_auto_import(&src, &home).unwrap();
        write_skill(&src, "late", "3"); // 后来在独立 dsh 里新增
        seed_auto_import(&src, &home).unwrap();
        assert!(home.join("skills/late/SKILL.md").is_file());
    }

    #[test]
    fn seed_skips_existing_without_overwrite() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        let home = t.path().join("home");
        write_skill(&src, "x", "new");
        write_skill(&home.join(DISABLED_DIR), "x", "mine"); // 壳里已有（含停用态）
        seed_auto_import(&src, &home).unwrap();
        let body = fs::read_to_string(home.join(format!("{DISABLED_DIR}/x/SKILL.md"))).unwrap();
        assert!(body.contains("mine"));
        assert!(!home.join("skills/x").exists());
    }

    #[test]
    fn seed_missing_source_is_noop() {
        let t = tempfile::tempdir().unwrap();
        seed_auto_import(&t.path().join("nope"), &t.path().join("home")).unwrap();
        assert!(!t.path().join("home/skills").exists());
    }
}
