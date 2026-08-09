//! OMP 内置命令读取模块。
//!
//! 从 `~/.omp/agent/builtin_commands.json` 和 `~/.omp/agent/commands/*.md`
//! 加载 OMP 的命令列表，用于在 CLI agent 输入框中替换 Zap 的静态命令。

use serde::Deserialize;
use std::path::PathBuf;

/// OMP 命令定义（来自 JSON 文件）
#[derive(Debug, Clone, Deserialize)]
pub struct OmpCommandDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub group: String,
    /// 提交该命令后是否把焦点切回终端 TUI（per-command 配置，缺省 false）
    #[serde(default)]
    pub focus_terminal_after_submit: bool,
}

/// OMP 命令清单（JSON 文件根结构）
#[derive(Debug, Deserialize)]
struct OmpCommandsFile {
    #[allow(dead_code)]
    version: Option<u32>,
    commands: Vec<OmpCommandDefinition>,
}

/// 合并后的 OMP 命令项
#[derive(Debug, Clone)]
pub struct OmpCommandItem {
    /// 命令文本，如 "/plan"
    pub text: String,
    /// 描述
    pub description: String,
    /// 来源："builtin" / "custom" / "skill"
    pub source: OmpCommandSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OmpCommandSource {
    Builtin,
    Custom,
    Skill,
}

/// OMP 配置目录：`~/.omp/agent/`
fn omp_agent_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".omp").join("agent"))
}

/// 读取 `~/.omp/agent/builtin_commands.json` 中的内置命令
fn load_builtin_commands() -> Vec<OmpCommandItem> {
    let Some(agent_dir) = omp_agent_dir() else {
        return vec![];
    };
    let path = agent_dir.join("builtin_commands.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read OMP builtin commands from {:?}: {e}", path);
            return vec![];
        }
    };
    let file: OmpCommandsFile = match serde_json::from_str(&content) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("Failed to parse OMP builtin commands: {e}");
            return vec![];
        }
    };
    file.commands
        .into_iter()
        .map(|cmd| OmpCommandItem {
            text: cmd.name,
            description: cmd.description,
            source: OmpCommandSource::Builtin,
        })
        .collect()
}

/// 读取 `~/.omp/agent/commands/*.md` 中的自定义命令
fn load_custom_commands() -> Vec<OmpCommandItem> {
    let Some(agent_dir) = omp_agent_dir() else {
        return vec![];
    };
    let commands_dir = agent_dir.join("commands");
    let dir = match std::fs::read_dir(&commands_dir) {
        Ok(d) => d,
        Err(_) => return vec![], // 目录不存在或不可读
    };

    let mut items = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_owned(),
            None => continue,
        };
        // 读取 frontmatter 提取 description
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let description = parse_frontmatter_description(&content)
            .unwrap_or_else(|| format!("Custom OMP command: /{stem}"));

        items.push(OmpCommandItem {
            text: format!("/{stem}"),
            description,
            source: OmpCommandSource::Custom,
        });
    }
    items
}

/// 从 markdown frontmatter 中提取 description 字段
fn parse_frontmatter_description(content: &str) -> Option<String> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")?;
    let frontmatter = &content[3..3 + end];
    for line in frontmatter.lines() {
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim().trim_matches('"').trim().to_owned();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// 获取所有 OMP 命令（内置 + 自定义），
/// 按来源排序：builtin → custom → skill。
/// `skill_items` 从外部传入，因为 skills 通过 SkillManager 加载。
pub fn all_omp_commands(skill_items: Vec<OmpCommandItem>) -> Vec<OmpCommandItem> {
    let mut commands = load_builtin_commands();
    commands.extend(load_custom_commands());
    commands.extend(skill_items);
    commands
}

/// 从 builtin_commands.json 内容解析提交 `command_text` 后是否把焦点切回 TUI。
/// 取首个空白分隔 token 匹配命令项 name;无匹配 / 属性缺失 / JSON 非法 → 默认 false。
fn resolve_focus_terminal_after_submit(content: &str, command_text: &str) -> bool {
    let file = match serde_json::from_str::<OmpCommandsFile>(content) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("Failed to parse OMP builtin commands config: {e}");
            return false;
        }
    };
    let command = command_text.split_whitespace().next().unwrap_or("");
    file.commands
        .iter()
        .find(|c| c.name == command)
        .map(|c| c.focus_terminal_after_submit)
        .unwrap_or(false)
}

/// 提交 `command_text` 后是否把焦点切回终端 TUI。文件/属性缺失 → 默认 false。
pub fn should_focus_terminal_after_submit(command_text: &str) -> bool {
    let Some(agent_dir) = omp_agent_dir() else {
        return false;
    };
    let path = agent_dir.join("builtin_commands.json");
    match std::fs::read_to_string(&path) {
        Ok(content) => resolve_focus_terminal_after_submit(&content, command_text),
        Err(e) => {
            log::warn!("Failed to read OMP builtin commands from {:?}: {e}", path);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_description() {
        let md = r#"---
description: Code-review a file or diff
---
Some content"#;
        assert_eq!(
            parse_frontmatter_description(md),
            Some("Code-review a file or diff".to_owned())
        );
    }

    #[test]
    fn test_parse_frontmatter_no_description() {
        let md = r#"---
other: value
---
Content"#;
        assert!(parse_frontmatter_description(md).is_none());
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let md = "Just content";
        assert!(parse_frontmatter_description(md).is_none());
    }

    #[test]
    fn test_resolve_focus_terminal_after_submit_per_command() {
        let content = r#"{"version":1,"commands":[
            {"name":"/resume","description":"d","group":"session","focus_terminal_after_submit":true},
            {"name":"/plan","description":"d","group":"top","focus_terminal_after_submit":false},
            {"name":"/model","description":"d","group":"top"}
        ]}"#;
        // 命令项显式 true
        assert!(resolve_focus_terminal_after_submit(content, "/resume"));
        // 命令项显式 false(与缺省是不同 serde 路径)
        assert!(!resolve_focus_terminal_after_submit(content, "/plan"));
        // 命令项缺省 → false
        assert!(!resolve_focus_terminal_after_submit(content, "/model"));
        // 带参数仍匹配命令名
        assert!(resolve_focus_terminal_after_submit(content, "/resume 019fbc91-a15f"));
        // 前导空白不影响命令名提取
        assert!(resolve_focus_terminal_after_submit(content, "  /resume"));
        // 非命令文本 → 默认 false
        assert!(!resolve_focus_terminal_after_submit(content, "fix the bug"));
    }

    #[test]
    fn test_resolve_focus_terminal_after_submit_defaults_false_on_failure() {
        assert!(!resolve_focus_terminal_after_submit("not json", "/resume"));
        assert!(!resolve_focus_terminal_after_submit("", "/resume"));
        assert!(!resolve_focus_terminal_after_submit(r#"{"version":1,"commands":[]}"#, "/resume"));
    }
}
