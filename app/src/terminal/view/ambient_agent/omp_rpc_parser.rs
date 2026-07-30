/// 行缓冲 NDJSON 解析器，用于解析 omp RPC 模式输出的 NDJSON 事件流。
///
/// # 解析策略
/// - 只解析完整的 NDJSON 行（`\n` 分隔）
/// - 忽略非 JSON 行（可以是混合输出、日志等）
/// - 解析错误静默忽略，不崩溃
use serde_json::Value;

/// omp RPC NDJSON 事件类型。
///
/// 对应 pi-coding-agent-core.el 的消息格式。
#[derive(Debug, Clone)]
pub(crate) enum OmpRpcEvent {
    /// `{"type":"assistant_message","content":"..."}`
    AssistantMessage { content: String },
    /// `{"type":"thinking","content":"..."}`
    Thinking { content: String },
    /// `{"type":"tool_use","name":"bash","input":{...}}`
    ToolUse {
        name: String,
        input: serde_json::Value,
    },
    /// `{"type":"tool_result","call_id":"...","output":"..."}`
    ToolResult {
        call_id: String,
        output: String,
    },
    /// `{"type":"error","code":"...","message":"..."}`
    Error {
        code: Option<String>,
        message: String,
    },
    /// `{"type":"status_change","status":"..."}`
    StatusChange { status: String },
}

/// 行缓冲 NDJSON 解析器。
pub(crate) struct OmpRpcParser {
    /// 行缓冲，存放不完整行
    buffer: String,
}

impl OmpRpcParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// 喂入原始终端输出文本，返回完整的已解析事件。
    ///
    /// - 只解析完整 NDJSON 行（`\n` 分隔）
    /// - 忽略无法解析的行（可能是混合输出、日志等）
    pub fn feed(&mut self, text: &str) -> Vec<OmpRpcEvent> {
        self.buffer.push_str(text);
        let mut events = Vec::new();
        loop {
            let Some(newline_idx) = self.buffer.find('\n') else {
                break;
            };
            let line = self.buffer[..newline_idx].trim().to_string();
            self.buffer.drain(..=newline_idx);
            if line.is_empty() {
                continue;
            }

            // 尝试解析为 NDJSON
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                let type_field = value.get("type").and_then(|v| v.as_str());
                match type_field {
                    Some("assistant_message") => {
                        if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
                            events.push(OmpRpcEvent::AssistantMessage {
                                content: content.to_string(),
                            });
                        }
                    }
                    Some("thinking") => {
                        if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
                            events.push(OmpRpcEvent::Thinking {
                                content: content.to_string(),
                            });
                        }
                    }
                    Some("tool_use") => {
                        let name = value
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let input = value.get("input").cloned().unwrap_or(Value::Null);
                        events.push(OmpRpcEvent::ToolUse { name, input });
                    }
                    Some("tool_result") => {
                        let call_id = value
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let output = value
                            .get("output")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        events.push(OmpRpcEvent::ToolResult { call_id, output });
                    }
                    Some("error") => {
                        let code = value.get("code").and_then(|v| v.as_str().map(String::from));
                        let message = value
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error")
                            .to_string();
                        events.push(OmpRpcEvent::Error { code, message });
                    }
                    Some("status_change") => {
                        if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
                            events.push(OmpRpcEvent::StatusChange {
                                status: status.to_string(),
                            });
                        }
                    }
                    _ => {
                        // 忽略未知类型
                    }
                }
            }
            // 非 JSON 行 → 忽略（混合输出安全容忍）
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_assistant_message() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed(r#"{"type":"assistant_message","content":"Hello"}"#);
        assert_eq!(events.len(), 1);
        match &events[0] {
            OmpRpcEvent::AssistantMessage { content } => assert_eq!(content, "Hello"),
            other => panic!("Expected AssistantMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_thinking() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed(r#"{"type":"thinking","content":"Let me think..."}"#);
        assert_eq!(events.len(), 1);
        match &events[0] {
            OmpRpcEvent::Thinking { content } => assert_eq!(content, "Let me think..."),
            other => panic!("Expected Thinking, got {other:?}"),
        }
    }

    #[test]
    fn parse_tool_use() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed(
            r#"{"type":"tool_use","name":"bash","input":{"command":"ls -la"}}"#,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            OmpRpcEvent::ToolUse { name, input } => {
                assert_eq!(name, "bash");
                assert_eq!(input["command"], "ls -la");
            }
            other => panic!("Expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn parse_tool_result() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed(
            r#"{"type":"tool_result","call_id":"call_123","output":"hello world"}"#,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            OmpRpcEvent::ToolResult { call_id, output } => {
                assert_eq!(call_id, "call_123");
                assert_eq!(output, "hello world");
            }
            other => panic!("Expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn parse_error() {
        let mut parser = OmpRpcParser::new();
        let events =
            parser.feed(r#"{"type":"error","code":"RATE_LIMITED","message":"Too many requests"}"#);
        assert_eq!(events.len(), 1);
        match &events[0] {
            OmpRpcEvent::Error { code, message } => {
                assert_eq!(code.as_deref(), Some("RATE_LIMITED"));
                assert_eq!(message, "Too many requests");
            }
            other => panic!("Expected Error, got {other:?}"),
        }
    }

    #[test]
    fn parse_status_change() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed(r#"{"type":"status_change","status":"done"}"#);
        assert_eq!(events.len(), 1);
        match &events[0] {
            OmpRpcEvent::StatusChange { status } => assert_eq!(status, "done"),
            other => panic!("Expected StatusChange, got {other:?}"),
        }
    }

    #[test]
    fn ignore_non_json_lines() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed("some random terminal output\n");
        assert!(events.is_empty());
    }

    #[test]
    fn parse_multiple_lines() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed(
            r#"{"type":"assistant_message","content":"First"}
{"type":"thinking","content":"Second"}
"#,
        );
        assert_eq!(events.len(), 2);
        match &events[0] {
            OmpRpcEvent::AssistantMessage { content } => assert_eq!(content, "First"),
            other => panic!("Expected AssistantMessage, got {other:?}"),
        }
        match &events[1] {
            OmpRpcEvent::Thinking { content } => assert_eq!(content, "Second"),
            other => panic!("Expected Thinking, got {other:?}"),
        }
    }

    #[test]
    fn partial_line_buffered_across_feeds() {
        let mut parser = OmpRpcParser::new();
        // First feed: incomplete line
        let events = parser.feed(r#"{"type":"assistant_mess"#);
        assert!(events.is_empty());

        // Second feed: rest of the line
        let events = parser.feed(r#"age","content":"Hi"}"#);
        assert!(events.is_empty());

        // Third feed: newline to trigger parsing
        let events = parser.feed("\n");
        assert_eq!(events.len(), 1);
        match &events[0] {
            OmpRpcEvent::AssistantMessage { content } => assert_eq!(content, "Hi"),
            other => panic!("Expected AssistantMessage, got {other:?}"),
        }
    }

    #[test]
    fn ignore_empty_lines() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed("\n\n\n");
        assert!(events.is_empty());
    }

    #[test]
    fn ignore_unknown_types() {
        let mut parser = OmpRpcParser::new();
        let events = parser.feed(r#"{"type":"unknown_thing","data":123}"#);
        assert!(events.is_empty());
    }
}
