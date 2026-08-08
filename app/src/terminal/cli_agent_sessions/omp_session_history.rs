//! 读取 omp 当前会话的 jsonl 记录,提取用户发送的消息。
//!
//! omp 把每个会话写为 `~/.omp/agent/sessions/-<cwd>/<ts>_<session_id>.jsonl`,
//! 每行一个 JSON 事件。用户消息形如:
//! ```json
//! {"type":"message", ..., "message":{"role":"user","content":[{"type":"text","text":"..."}]}}
//! ```
//!
//! 注意:omp 的 OSC777 `session_id` 是运行实例 id,与 jsonl 文件名中的会话存储 id
//! 并非同一体系(resume 旧会话时继续写历史 jsonl,但上报新 id)。因此定位分两级:
//! 先按 session_id 匹配文件,失败则用 cwd 查 `terminal-sessions/ttys*` 映射。
//!
//! 本模块只读不写:Zap 不持久化任何内容,按需从磁盘读取当前会话的用户消息;
//! 找不到会话文件或解析失败时返回空(自然降级,不影响其它历史)。

use chrono::{DateTime, Local};
use serde_json::Value;

/// omp 会话中的一条用户消息。
#[derive(Debug, Clone)]
pub struct OmpUserMessage {
    pub text: String,
    pub timestamp: DateTime<Local>,
}

/// 读取 omp 会话(`session_id` 为 OSC777 `session_start` 上报的 id)的用户消息列表,
/// 读取 omp 当前会话的用户消息列表,按文件顺序(即时间序)返回。
///
/// 返回 `Some(messages)` 表示定位到了 omp 会话(新会话文件可能尚未落盘,
/// 此时为空列表);返回 `None` 表示完全无法定位(非 omp / 无映射),调用方
/// 回退到其它历史来源。
///
/// 定位策略(omp 的 OSC777 `session_id` 是运行实例 id,与 jsonl 文件名中的
/// 会话存储 id 并非同一体系,所以需要两级定位):
/// 1. 若 `session_id` 能匹配到 `~/.omp/agent/sessions/*/*_<id>.jsonl`,直接解析;
/// 2. 否则用 `~/.omp/agent/terminal-sessions/ttys*` 映射(文件内容首行为会话
///    cwd,次行为 jsonl 路径,omp 在新建/恢复会话时更新)。注意:新会话时
///    omp 先更新映射、jsonl 惰性落盘,因此映射命中就采用(空文件 = 空会话),
///    不能因文件缺失回退到其它会话的旧历史。
#[cfg(not(target_family = "wasm"))]
pub fn read_omp_user_messages(
    session_id: Option<&str>,
    cwd: Option<&str>,
) -> Option<Vec<OmpUserMessage>> {
    let Some(home) = dirs::home_dir() else {
        return None;
    };
    let omp_root = home.join(".omp/agent");

    if let Some(session_id) = session_id.filter(|s| !s.is_empty()) {
        if let Some(path) = find_session_file_by_id(&omp_root.join("sessions"), session_id) {
            // 文件存在:解析失败(损坏/不可读)返回 None,让调用方回退其它历史。
            return parse_omp_session_file(&path);
        }
    }

    if let Some(path) =
        resolve_session_file_via_tty_mapping(&omp_root.join("terminal-sessions"), cwd)
    {
        return if path.is_file() {
            parse_omp_session_file(&path)
        } else {
            // 映射命中但 jsonl 尚未落盘(omp 新会话惰性写文件):空会话。
            Some(Vec::new())
        };
    }

    None
}

/// 在会话根目录下按 `<ts>_<session_id>.jsonl` 后缀定位文件。
#[cfg(not(target_family = "wasm"))]
fn find_session_file_by_id(
    sessions_dir: &std::path::Path,
    session_id: &str,
) -> Option<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return None;
    };

    let target_suffix = format!("_{session_id}.jsonl");
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.ends_with(&target_suffix) {
                return Some(path);
            }
        }
    }
    None
}

/// 用 cwd 匹配 omp 的 `terminal-sessions/ttys*` 映射,返回 jsonl 路径。
///
/// 映射文件内容:首行 cwd、次行 jsonl 路径(omp 在新建/恢复会话时更新映射)。
/// 定位策略:
/// 1. cwd 匹配的所有映射中,取**映射文件 mtime** 最新的——最近切换过会话的
///    终端就是当前会话(新会话 jsonl 可能尚未落盘,jsonl mtime 不可用作排名);
/// 2. cwd 无匹配时,同样取映射文件 mtime 最新的(omp 的 OSC777 session_id 与
///    jsonl 文件名不是同一体系,也没有可靠的 cwd,这是最实用的回退)。
#[cfg(not(target_family = "wasm"))]
fn resolve_session_file_via_tty_mapping(
    terminal_sessions_dir: &std::path::Path,
    cwd: Option<&str>,
) -> Option<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(terminal_sessions_dir) else {
        return None;
    };

    // (jsonl 路径, 映射文件 mtime, 映射 cwd)
    let mut all: Vec<(std::path::PathBuf, std::time::SystemTime, Option<String>)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut lines = content.lines();
        let map_cwd = lines.next().map(str::to_owned);
        let Some(jsonl) = lines.next() else {
            continue;
        };
        let jsonl_path = std::path::PathBuf::from(jsonl);
        // 注意:jsonl 可能尚未落盘(omp 新会话惰性写文件),映射命中即采用,
        // 不在此处用 is_file 过滤——否则会回退到其它会话的旧历史。
        let mapping_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if let Some(mtime) = mapping_mtime {
            all.push((jsonl_path, mtime, map_cwd));
        }
    }

    // 1) cwd 精确匹配优先;命中多个时取映射文件 mtime 最新(最近切到该 cwd
    //    会话的终端,新会话 jsonl 未落盘时 mtime 也是 None,不能用来排名)。
    let by_cwd: Vec<_> = all
        .iter()
        .filter(|(_, _, map_cwd)| cwd.is_some_and(|c| map_cwd.as_deref() == Some(c)))
        .map(|(jsonl, mapping_mtime, _)| (jsonl.clone(), *mapping_mtime))
        .collect();
    if !by_cwd.is_empty() {
        return by_cwd
            .into_iter()
            .max_by_key(|(_, mapping_mtime)| *mapping_mtime)
            .map(|(jsonl, _)| jsonl);
    }

    // 2) 兜底:映射文件 mtime 最新(最近切换过会话的终端大概率是当前操作的)。
    all.into_iter()
        .max_by_key(|(_, mtime, _)| *mtime)
        .map(|(jsonl, _, _)| jsonl)
}

/// WASM 上没有本地 omp 会话文件,无法定位,返回 `None`(回退其它历史来源)。
#[cfg(target_family = "wasm")]
pub fn read_omp_user_messages(
    _session_id: Option<&str>,
    _cwd: Option<&str>,
) -> Option<Vec<OmpUserMessage>> {
    None
}

/// 解析 omp 会话文件,提取用户消息。
///
/// 返回 `None` 表示文件存在但读取失败(损坏/权限等)——调用方应回退其它历史,
/// 不能当作空会话;返回 `Some(vec)` 表示解析成功(新会话未落盘时文件不存在,
/// 由调用方在映射命中时直接视为空会话,不调用本函数)。
#[cfg(not(target_family = "wasm"))]
fn parse_omp_session_file(path: &std::path::Path) -> Option<Vec<OmpUserMessage>> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return None;
    };

    let mut messages = Vec::new();
    // 缺失 timestamp 的行沿用上一条已解析消息的时间戳,保持排序稳定
    // (真实 omp 数据每条都有 timestamp,该兜底几乎不可达)。
    let mut last_timestamp: Option<DateTime<Local>> = None;
    for line in content.lines() {
        // 快速跳过非 message 行(assistant/tool/自定义事件是大头,避免全量 serde 解析)。
        if !line.starts_with("{\"type\":\"message\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }

        // 拼接 content 里的 text 块(用户消息通常为单个 text 块)。
        let mut text = String::new();
        let mut found_text = false;
        if let Some(blocks) = message.get("content").and_then(Value::as_array) {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                        found_text = true;
                    }
                }
            }
        }
        if !found_text {
            continue;
        }
        let text = text.trim();
        if text.is_empty() {
            continue;
        }

        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Local))
            .or(last_timestamp)
            .unwrap_or_else(Local::now);
        last_timestamp = Some(timestamp);

        messages.push(OmpUserMessage {
            text: text.to_string(),
            timestamp,
        });
    }
    Some(messages)
}

#[cfg(test)]
#[path = "omp_session_history_tests.rs"]
mod tests;
