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
}
