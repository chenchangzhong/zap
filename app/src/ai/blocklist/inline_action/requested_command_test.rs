//! Unit tests for format_command_text in requested_command.rs

use super::{format_command_text, mcp_blocked_title_text, mcp_viewing_detail_title_text};

#[test]
fn single_line_without_newline_is_unchanged_ascii() {
    let input = "echo hello world";
    let output = format_command_text(input);
    assert_eq!(output, input);
}

#[test]
fn single_line_without_newline_preserves_multibyte_characters() {
    let input = "echo 🚀✨";
    let output = format_command_text(input);
    assert_eq!(output, input);

    // Additional sanity check: string is valid UTF-8 and can be iterated by chars without panic
    let collected: String = output.chars().collect();
    assert_eq!(collected, output);
}

#[test]
fn truncates_at_first_newline_and_appends_ellipsis_when_more_content_exists() {
    let input = "cargo build\n--release";
    let output = format_command_text(input);
    assert_eq!(output, "cargo build…");
}

#[test]
fn truncates_at_first_newline_without_ellipsis_when_rest_is_whitespace() {
    let input = "git status\n   \t  ";
    let output = format_command_text(input);
    assert_eq!(output, "git status");
}

#[test]
fn does_not_split_multibyte_char_across_utf8_boundaries_when_newline_follows() {
    // The emoji is a multi-byte sequence; ensure truncation at the newline does not split it.
    let input = "echo 🧪\nthen do something";
    let output = format_command_text(input);
    assert_eq!(output, "echo 🧪…");

    // Validate resulting string is valid UTF-8 by iterating graphemes via chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn preserves_combining_characters_when_newline_is_after_cluster() {
    // "e" + combining acute accent
    // Sanity checks that the formatter doesn't split this unicode sequence
    let composed = format!("{}{}", 'e', '\u{0301}');
    let input = format!("echo {composed}\nnext");
    let output = format_command_text(&input);
    assert_eq!(output, format!("echo {composed}…"));

    // Still valid UTF-8 and same when re-collected from chars
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

#[test]
fn newline_then_multibyte_results_in_ellipsis_only() {
    let input = "\n🚀";
    let output = format_command_text(input);
    assert_eq!(output, "…");

    // Sanity: output remains valid UTF-8
    let reconstructed: String = output.chars().collect();
    assert_eq!(reconstructed, output);
}

/// 这些标题已改为走 i18n(见 `blocked-action-call-mcp-tool*` 系列键),所以断言
/// 的是**插值行为**(工具名/服务器名必须出现,已知服务器与未知服务器要给出不同
/// 文案),而不是某一个 locale 的具体措辞。
#[test]
fn mcp_blocked_title_surfaces_tool_and_server_when_known() {
    crate::i18n::init(Some("en"));
    let with_server = mcp_blocked_title_text("create_issue", Some("github"));
    assert!(
        with_server.contains("create_issue"),
        "工具名缺失: {with_server}"
    );
    assert!(
        with_server.contains("github"),
        "服务器名缺失: {with_server}"
    );
    assert!(!with_server.contains("$"), "参数占位符残留: {with_server}");

    let without_server = mcp_blocked_title_text("create_issue", None);
    assert!(
        without_server.contains("create_issue"),
        "工具名缺失: {without_server}"
    );
    assert!(
        !without_server.contains("github"),
        "未提供服务器时不应出现服务器名: {without_server}"
    );
    assert_ne!(
        with_server, without_server,
        "已知服务器与未知服务器必须给出不同文案"
    );
}

/// 文案必须来自 i18n(而不是旧硬编码英文)。
///
/// 注意:不能在这个用例里切换 locale 来比较 —— `i18n::init()` 是 `OnceLock`,
/// 首次调用后永远幂等(见其文档),测试进程内先跑到的 locale 会定调。所以这里
/// 只断言「不是硬编码英文原文」「不含未解析的 key」。
#[test]
fn mcp_blocked_title_is_localized() {
    crate::i18n::init(Some("en"));
    let title = mcp_blocked_title_text("create_issue", Some("github"));
    assert!(title.contains("create_issue"), "工具名缺失: {title}");
    assert!(
        !title.contains("agent-action-name") && !title.contains("blocked-action-"),
        "i18n 键未解析: {title}"
    );
    assert_ne!(
        title, "OK if I call MCP tool create_issue on server github",
        "不应再返回硬编码英文原文(说明没走 i18n): {title}"
    );
}

#[test]
fn mcp_blocked_title_falls_back_to_generic_message_when_tool_name_empty() {
    crate::i18n::init(Some("en"));
    let with_server = mcp_blocked_title_text("", Some("github"));
    let without_server = mcp_blocked_title_text("", None);
    assert_eq!(
        with_server, without_server,
        "工具名缺失时应回退到同一条通用文案"
    );
    assert!(!with_server.contains("$"), "参数占位符残留: {with_server}");
    assert!(!with_server.is_empty(), "通用文案不得为空");
}

#[test]
fn mcp_viewing_detail_title_surfaces_tool_and_server_when_known() {
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", Some("github")),
        "Viewing MCP tool create_issue on github"
    );
    assert_eq!(
        mcp_viewing_detail_title_text("create_issue", None),
        "Viewing MCP tool create_issue"
    );
}

#[test]
fn mcp_viewing_detail_title_falls_back_to_generic_message_when_tool_name_empty() {
    assert_eq!(
        mcp_viewing_detail_title_text("", Some("github")),
        "Viewing MCP tool call detail"
    );
}
