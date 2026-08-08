use super::{
    find_session_file_by_id, parse_omp_session_file, resolve_session_file_via_tty_mapping,
};

#[test]
fn parses_user_messages_from_omp_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-08-08T05:09:36.914Z"}
{"type":"message","id":"1","parentId":null,"timestamp":"2026-08-08T05:10:13.526Z","message":{"role":"user","content":[{"type":"text","text":"hello world"}],"attribution":"user","timestamp":1786165812882}}
{"type":"message","id":"2","parentId":"1","timestamp":"2026-08-08T05:10:22.181Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"..."}]}}
{"type":"message","id":"3","parentId":"2","timestamp":"2026-08-08T05:11:00.000Z","message":{"role":"user","content":[{"type":"text","text":"  带空白的消息  "}],"attribution":"user"}}
{"type":"message","id":"4","parentId":"3","timestamp":"2026-08-08T05:11:01.000Z","message":{"role":"user","content":[{"type":"text","text":"   "}]}}
{"type":"custom","customType":"tool_execution_start","id":"5","timestamp":"2026-08-08T05:11:02.000Z"}
"#,
    )
    .unwrap();

    let messages = parse_omp_session_file(&path).expect("file parses");
    assert_eq!(messages.len(), 2);

    assert_eq!(messages[0].text, "hello world");
    // 时间戳保留原始时刻(转回 UTC 校验,与本地时区无关)。
    assert!(
        messages[0]
            .timestamp
            .with_timezone(&chrono::Utc)
            .to_rfc3339()
            .starts_with("2026-08-08T05:10:13"),
        "unexpected timestamp: {}",
        messages[0].timestamp.to_rfc3339()
    );

    // 用户消息带首尾空白时 trim,时间戳缺失时兜底(此处校验非空即可)。
    assert_eq!(messages[1].text, "带空白的消息");
    assert!(!messages[1].timestamp.to_rfc3339().is_empty());
}

#[test]
fn returns_none_for_missing_files_and_empty_for_broken_lines() {
    let dir = tempfile::tempdir().unwrap();

    // 不存在的文件:读取失败 → None(调用方回退其它历史)。
    assert!(parse_omp_session_file(&dir.path().join("missing.jsonl")).is_none());

    // 损坏的行直接跳过:文件可读但无有效消息 → Some(空)。
    let path = dir.path().join("broken.jsonl");
    std::fs::write(&path, "not json\n{\"type\":\"message\"}\n").unwrap();
    let messages = parse_omp_session_file(&path).expect("file is readable");
    assert!(messages.is_empty());
}

#[test]
fn find_session_file_matches_session_id_suffix() {
    // 用真实布局:多个项目目录,目标 session_id 只在一个文件里。
    let dir = tempfile::tempdir().unwrap();
    let sessions_root = dir.path().join("sessions");
    let proj = sessions_root.join("-project-zap");
    std::fs::create_dir_all(&proj).unwrap();
    let target = proj.join("2026-08-08T05-09-36-914Z_019fdfc6-d212-7000-b1e2-756b90226c0e.jsonl");
    std::fs::write(
        &target,
        r#"{"type":"message","id":"1","timestamp":"2026-08-08T05:10:13.526Z","message":{"role":"user","content":[{"type":"text","text":"the message"}]}}
"#,
    )
    .unwrap();
    std::fs::write(
        proj.join("2026-08-07T03-10-45-949Z_019fda33-a6bd-7000-ab6d-0757bb5141fa.jsonl"),
        r#"{"type":"message","id":"1","timestamp":"2026-08-07T03:10:45.949Z","message":{"role":"user","content":[{"type":"text","text":"old message"}]}}
"#,
    )
    .unwrap();

    let found = find_session_file_by_id(&sessions_root, "019fdfc6-d212-7000-b1e2-756b90226c0e");
    assert_eq!(found.as_deref(), Some(target.as_path()));

    // 未知 session_id 返回 None。
    assert!(find_session_file_by_id(&sessions_root, "does-not-exist").is_none());
    // 根目录不存在返回 None。
    assert!(find_session_file_by_id(&dir.path().join("nope"), "x").is_none());
}

#[test]
fn tty_mapping_resolves_by_cwd_and_newest_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let tty_dir = dir.path().join("terminal-sessions");
    std::fs::create_dir_all(&tty_dir).unwrap();

    // 两个 tty 同 cwd:一个指向旧 jsonl,一个指向新 jsonl。
    let old_jsonl = dir.path().join("old.jsonl");
    let new_jsonl = dir.path().join("new.jsonl");
    std::fs::write(&old_jsonl, r#"{"type":"message","id":"1","timestamp":"2026-08-08T05:10:13.526Z","message":{"role":"user","content":[{"type":"text","text":"old tty message"}]}}
"#)
    .unwrap();
    std::fs::write(&new_jsonl, r#"{"type":"message","id":"1","timestamp":"2026-08-08T05:10:13.526Z","message":{"role":"user","content":[{"type":"text","text":"new tty message"}]}}
"#)
    .unwrap();
    // 确保 new_jsonl mtime 更新。
    let new_time = std::fs::metadata(&new_jsonl).unwrap().modified().unwrap();
    std::fs::write(
        tty_dir.join("ttys001"),
        format!("/proj\n{}\nfresh\n", old_jsonl.display()),
    )
    .unwrap();
    std::fs::write(
        tty_dir.join("ttys002"),
        format!("/proj\n{}\nfresh\n", new_jsonl.display()),
    )
    .unwrap();
    // 不同 cwd 的映射不匹配。
    std::fs::write(
        tty_dir.join("ttys003"),
        format!("/other\n{}\nfresh\n", old_jsonl.display()),
    )
    .unwrap();

    // 时间戳打平:直接触摸 new_jsonl 使 mtime 更新。
    let file = std::fs::File::options().write(true).open(&new_jsonl).unwrap();
    file.set_modified(new_time + std::time::Duration::from_secs(10))
        .unwrap();
    drop(file);

    let resolved = resolve_session_file_via_tty_mapping(&tty_dir, Some("/proj"));
    assert_eq!(resolved.as_deref(), Some(new_jsonl.as_path()));

    // cwd 无匹配时回退到映射文件 mtime 最新(ttys003 最晚写入)。
    std::fs::write(
        tty_dir.join("ttys003"),
        format!("/other\n{}\nfresh\n", old_jsonl.display()),
    )
    .unwrap();
    let resolved_fallback = resolve_session_file_via_tty_mapping(&tty_dir, Some("/nope"));
    assert_eq!(resolved_fallback.as_deref(), Some(old_jsonl.as_path()));

    // 目录不存在返回 None。
    assert!(resolve_session_file_via_tty_mapping(&dir.path().join("nope"), Some("/proj")).is_none());
}

#[test]
fn tty_mapping_honors_latest_mapping_even_when_jsonl_not_yet_created() {
    // omp 新会话:映射已指向尚未落盘的 jsonl。cwd 匹配必须按映射文件 mtime
    // 排名(最近切换的终端),不能因 jsonl 缺失选到其它会话的旧文件。
    let dir = tempfile::tempdir().unwrap();
    let tty_dir = dir.path().join("terminal-sessions");
    std::fs::create_dir_all(&tty_dir).unwrap();

    let old_jsonl = dir.path().join("old.jsonl");
    std::fs::write(&old_jsonl, "x").unwrap();
    std::fs::write(tty_dir.join("ttys001"), format!("/proj\n{}\nfresh\n", old_jsonl.display()))
        .unwrap();

    // 新映射(更新,指向未落盘的 jsonl)。
    let pending_jsonl = dir.path().join("pending.jsonl");
    std::fs::write(
        tty_dir.join("ttys002"),
        format!("/proj\n{}\nfresh\n", pending_jsonl.display()),
    )
    .unwrap();

    // cwd 匹配:两个映射同 cwd,ttys002 映射文件 mtime 更新 → 选 pending(新会话,空)。
    let resolved = resolve_session_file_via_tty_mapping(&tty_dir, Some("/proj"));
    assert_eq!(resolved.as_deref(), Some(pending_jsonl.as_path()));

    // 无 cwd 匹配(如 OSC777/ActiveSession cwd 不可用):同样取映射文件 mtime 最新。
    let resolved_fallback = resolve_session_file_via_tty_mapping(&tty_dir, None);
    assert_eq!(resolved_fallback.as_deref(), Some(pending_jsonl.as_path()));

    // 目录不存在返回 None。
    assert!(resolve_session_file_via_tty_mapping(&dir.path().join("nope"), Some("/proj")).is_none());
}

#[test]
fn user_message_without_text_block_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"message","id":"1","timestamp":"2026-08-08T05:10:13.526Z","message":{"role":"user","content":[{"type":"tool_use","name":"read"}]}}
"#,
    )
    .unwrap();
    let messages = parse_omp_session_file(&path).expect("file is readable");
    assert!(messages.is_empty());
}
