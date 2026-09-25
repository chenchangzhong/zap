use super::*;

#[test]
fn test_parse_range_with_comma() {
    let (start, count) = DiffStateModel::parse_range("10,5")
        .expect("parse_range should succeed for range with count");
    assert_eq!(start, 10);
    assert_eq!(count, 5);
}

#[test]
fn test_parse_range_without_comma() {
    let (start, count) = DiffStateModel::parse_range("10")
        .expect("parse_range should succeed for range without count");
    assert_eq!(start, 10);
    assert_eq!(count, 1);
}

#[test]
fn test_parse_unified_diff_header_basic() {
    let header = "@@ -10,5 +12,7 @@";
    let parsed = DiffStateModel::parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for basic header");
    assert_eq!(parsed.old_start_line, 10);
    assert_eq!(parsed.old_line_count, 5);
    assert_eq!(parsed.new_start_line, 12);
    assert_eq!(parsed.new_line_count, 7);
}

#[test]
fn test_parse_unified_diff_header_with_context() {
    let header = "@@ -4978,33 +4978,43 @@ impl TerminalView {";
    let parsed = DiffStateModel::parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for header with context");
    assert_eq!(parsed.old_start_line, 4978);
    assert_eq!(parsed.old_line_count, 33);
    assert_eq!(parsed.new_start_line, 4978);
    assert_eq!(parsed.new_line_count, 43);
}

#[test]
fn test_parse_unified_diff_header_single_line() {
    let header = "@@ -10 +12,3 @@";
    let parsed = DiffStateModel::parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for single line header");
    assert_eq!(parsed.old_start_line, 10);
    assert_eq!(parsed.old_line_count, 1);
    assert_eq!(parsed.new_start_line, 12);
    assert_eq!(parsed.new_line_count, 3);
}

#[test]
fn test_sort_branches_main_first_empty() {
    let branches: Vec<(String, bool)> = vec![];
    let result: Vec<_> = DiffStateModel::sort_branches_main_first(&branches).collect();
    assert!(result.is_empty());
}

#[test]
fn test_sort_branches_main_first_no_main() {
    let branches = vec![
        ("feature-a".to_string(), false),
        ("feature-b".to_string(), false),
        ("feature-c".to_string(), false),
    ];
    let result: Vec<_> = DiffStateModel::sort_branches_main_first(&branches).collect();
    // No main branches — order should be unchanged.
    assert_eq!(result, branches.iter().collect::<Vec<_>>());
}

#[test]
fn test_sort_branches_main_first_promotes_main() {
    let branches = vec![
        ("feature-a".to_string(), false),
        ("main".to_string(), true),
        ("feature-b".to_string(), false),
    ];
    let result: Vec<_> = DiffStateModel::sort_branches_main_first(&branches)
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(result, vec!["main", "feature-a", "feature-b"]);
}

#[test]
fn test_sort_branches_main_first_main_already_first() {
    let branches = vec![
        ("main".to_string(), true),
        ("feature-a".to_string(), false),
        ("feature-b".to_string(), false),
    ];
    let result: Vec<_> = DiffStateModel::sort_branches_main_first(&branches)
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(result, vec!["main", "feature-a", "feature-b"]);
}

#[test]
fn test_sort_branches_main_first_preserves_recency_order_for_non_main() {
    // Non-main branches should remain in their original (recency) order.
    let branches = vec![
        ("recent-feature".to_string(), false),
        ("main".to_string(), true),
        ("older-feature".to_string(), false),
        ("oldest-feature".to_string(), false),
    ];
    let result: Vec<_> = DiffStateModel::sort_branches_main_first(&branches)
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        result,
        vec!["main", "recent-feature", "older-feature", "oldest-feature"]
    );
}

#[test]
fn test_sort_branches_main_first_multiple_main_flags() {
    // Defensive: both flagged as main (shouldn't happen in practice, but
    // sort_branches_main_first should handle it gracefully).
    let branches = vec![
        ("feature".to_string(), false),
        ("main".to_string(), true),
        ("master".to_string(), true),
    ];
    let result: Vec<_> = DiffStateModel::sort_branches_main_first(&branches)
        .map(|(name, _)| name.as_str())
        .collect();
    // Both main-flagged entries appear first, non-main last.
    assert_eq!(result, vec!["main", "master", "feature"]);
}

#[test]
fn test_parse_unified_diff_header_malformed() {
    let header = "not a diff header";
    let result = DiffStateModel::parse_unified_diff_header(header);
    assert!(result.is_err());

    let header2 = "@@ incomplete";
    let result2 = DiffStateModel::parse_unified_diff_header(header2);
    assert!(result2.is_err());
}

#[test]
fn test_parse_git_status_modified_file_with_spaces() {
    // Porcelain v2 output for a modified file with spaces in the name.
    // Format: 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 test file.txt";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, std::path::PathBuf::from("test file.txt"));
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[test]
fn test_parse_git_status_modified_file_with_multiple_spaces() {
    // Filename with multiple spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 path to/my test file.txt";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].0,
        std::path::PathBuf::from("path to/my test file.txt")
    );
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[test]
fn test_parse_git_status_new_file_with_spaces() {
    let status_output = "1 A. N... 000000 100644 100644 0000000 abc1234 new file name.rs";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, std::path::PathBuf::from("new file name.rs"));
    assert_eq!(result[0].1, GitFileStatus::New);
}

#[test]
fn test_parse_git_status_renamed_file_with_spaces() {
    // Porcelain v2 renamed entry (type 2) with spaces in the new path.
    // Format: 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>
    let status_output =
        "2 R. N... 100644 100644 100644 abc1234 def5678 R100 new name.txt\0old name.txt";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, std::path::PathBuf::from("new name.txt"));
    assert!(matches!(
        &result[0].1,
        GitFileStatus::Renamed { old_path } if old_path == "old name.txt"
    ));
}

#[test]
fn test_parse_git_status_untracked_file_with_spaces() {
    let status_output = "? my untracked file.txt";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].0,
        std::path::PathBuf::from("my untracked file.txt")
    );
    assert_eq!(result[0].1, GitFileStatus::Untracked);
}

#[test]
fn test_parse_git_status_unmerged_file_with_spaces() {
    // Porcelain v2 unmerged entry (type u) with spaces in the path.
    // Format: u <xy> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
    let status_output =
        "u UU N... 100644 100644 100644 100644 abc1234 def5678 ghi9012 conflict file.txt";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, std::path::PathBuf::from("conflict file.txt"));
    assert_eq!(result[0].1, GitFileStatus::Conflicted);
}

#[test]
fn test_parse_git_status_mixed_entries_with_spaces() {
    // Multiple entries separated by NUL, mixing files with and without spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 test file.txt\0\
         1 .M N... 100644 100644 100644 abc1234 def5678 normal.txt\0\
         ? another file with spaces.rs";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].0, std::path::PathBuf::from("test file.txt"));
    assert_eq!(result[1].0, std::path::PathBuf::from("normal.txt"));
    assert_eq!(
        result[2].0,
        std::path::PathBuf::from("another file with spaces.rs")
    );
}

#[test]
fn test_parse_git_status_file_without_spaces_still_works() {
    // Ensure the splitn change doesn't break files without spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 simple.txt";
    let result = DiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, std::path::PathBuf::from("simple.txt"));
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[tokio::test]
async fn untracked_directory_diff_is_empty_and_non_binary() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::create_dir(repo_dir.path().join("nested-repo")).expect("create nested dir");

    // `git status` 会把嵌套 repo/worktree 报告为单个 untracked 目录条目(带尾斜杠)。
    // 它必须短路为空非二进制 diff —— 错误回退否则会误标为二进制,视图会渲染
    // "Binary file - no diff available" 而不是 "New empty file"。
    let diff = DiffStateModel::get_file_diff(
        repo_dir.path(),
        &std::path::PathBuf::from("nested-repo/"),
        &GitFileStatus::Untracked,
        false,
        None,
    )
    .await
    .expect("get_file_diff should succeed for an untracked directory");

    assert!(!diff.is_binary);
    assert_eq!(diff.hunks.len(), 0);
    assert_eq!(diff.status, GitFileStatus::Untracked);
}

/// 未跟踪的二进制文件不再 spawn `git diff --no-index`:其 diff 结果恒为「无 hunks 的
/// 二进制」,本地判据即可得出。这里锁定该固定结构(必须与 git 路径返回的字段完全一致,
/// 否则视图/统计会漂移)。
#[tokio::test]
async fn untracked_binary_file_short_circuits_to_fixed_binary_diff() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::create_dir(repo_dir.path().join("assets")).expect("create assets dir");
    // NUL 出现在 1024 字节之后、8000 字节之内:仍须判为二进制(git 的窗口就是 8000)。
    let mut content = vec![b'a'; 5000];
    content.push(0);
    content.extend_from_slice(b"tail");
    std::fs::write(repo_dir.path().join("assets/image.png"), content).expect("write binary file");

    let diff = DiffStateModel::get_file_diff(
        repo_dir.path(),
        &std::path::PathBuf::from("assets/image.png"),
        &GitFileStatus::Untracked,
        false,
        None,
    )
    .await
    .expect("get_file_diff should succeed for an untracked binary file");

    assert!(diff.is_binary);
    assert_eq!(diff.hunks.len(), 0);
    assert_eq!(diff.max_line_number, 0);
    assert!(!diff.has_hidden_bidi_chars);
    assert!(!diff.is_autogenerated);
    assert_eq!(diff.size, DiffSize::Normal);
    assert_eq!(diff.status, GitFileStatus::Untracked);
}

/// 文本未跟踪文件不受短路影响,仍要拿到「全部新增行」的 hunks。
#[tokio::test]
async fn untracked_text_file_still_produces_added_hunks() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::write(repo_dir.path().join("new-file.txt"), "one\ntwo\nthree\n")
        .expect("write text file");

    let diff = DiffStateModel::get_file_diff(
        repo_dir.path(),
        &std::path::PathBuf::from("new-file.txt"),
        &GitFileStatus::Untracked,
        false,
        None,
    )
    .await
    .expect("get_file_diff should succeed for an untracked text file");

    assert!(!diff.is_binary);
    assert_eq!(diff.additions(), 3);
    assert_eq!(diff.deletions(), 0);
}

/// 空未跟踪文件绝不能被判成二进制 —— 否则视图渲染 "Binary file - no diff available",
/// 而正确结果是 "New empty file"。`touch newfile` 是极常见操作,这条必须锁住。
#[tokio::test]
async fn empty_untracked_file_is_not_binary() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::write(repo_dir.path().join("empty.txt"), "").expect("write empty file");

    let diff = DiffStateModel::get_file_diff(
        repo_dir.path(),
        &std::path::PathBuf::from("empty.txt"),
        &GitFileStatus::Untracked,
        false,
        None,
    )
    .await
    .expect("get_file_diff should succeed for an empty untracked file");

    assert!(!diff.is_binary);
    assert_eq!(diff.hunks.len(), 0);
}

/// 本地判据必须与 git 的 `buffer_is_binary` 同窗口(前 8000 字节),且读取失败时保守回退。
#[test]
fn is_binary_file_content_matches_git_window() {
    let dir = tempfile::tempdir().expect("create temp dir");

    let text_path = dir.path().join("text.txt");
    std::fs::write(&text_path, "hello\n").expect("write text");
    assert!(!DiffStateModel::is_binary_file_content(&text_path));

    let empty_path = dir.path().join("empty.txt");
    std::fs::write(&empty_path, "").expect("write empty");
    assert!(!DiffStateModel::is_binary_file_content(&empty_path));

    // 窗口内的 NUL(第 5001 字节)-> 二进制。
    let late_nul_path = dir.path().join("late.bin");
    let mut content = vec![b'a'; 5000];
    content.push(0);
    std::fs::write(&late_nul_path, content).expect("write late-nul file");
    assert!(DiffStateModel::is_binary_file_content(&late_nul_path));

    // 窗口外的 NUL(第 9001 字节)-> 不判定,交给 git(宁可多一次 spawn,不可误判)。
    let far_nul_path = dir.path().join("far.bin");
    let mut content = vec![b'a'; 9000];
    content.push(0);
    std::fs::write(&far_nul_path, content).expect("write far-nul file");
    assert!(!DiffStateModel::is_binary_file_content(&far_nul_path));

    // 读不到 -> false(保守回退)。
    assert!(!DiffStateModel::is_binary_file_content(
        &dir.path().join("missing")
    ));
}

/// 解析 `cat-file --batch` 的分帧:正常 blob / 缺失 / **树(带内容段,必须跳过)** / 空 blob。
#[test]
fn parse_cat_file_batch_output_handles_missing_tree_and_empty_blob() {
    let output = b"abc123 blob 5\nhello\nHEAD:gone missing\ndef456 tree 3\nxyz\nghi789 blob 0\n\n";
    let parsed = DiffStateModel::parse_cat_file_batch_output(output, 4);
    assert_eq!(
        parsed,
        vec![Some("hello".to_string()), None, None, Some(String::new())]
    );
}

/// 截断的输出不能 panic,缺的条目补 None。
#[test]
fn parse_cat_file_batch_output_tolerates_truncated_stream() {
    let output = b"abc123 blob 99\nhel";
    let parsed = DiffStateModel::parse_cat_file_batch_output(output, 2);
    assert_eq!(parsed, vec![None, None]);
}

/// 批量取 baseline 必须与逐个 `git show HEAD:<path>` **完全一致** —— 用原路径当 oracle。
/// 覆盖:修改 / 删除 / 重命名(内容在旧路径) / 已 add 的新文件 / 未跟踪。
#[tokio::test]
async fn batched_baselines_match_per_file_git_show() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let run_git = |args: &[&str]| {
        let output = command::blocking::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "test@test.com"]);
    run_git(&["config", "user.name", "Test"]);
    for (name, content) in [
        ("modified.txt", "v1\n"),
        ("deleted.txt", "bye\n"),
        ("renamed.txt", "old\n"),
    ] {
        std::fs::write(dir.path().join(name), content).expect("write fixture");
    }
    run_git(&["add", "."]);
    run_git(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", "init"]);

    // 制造各类改动;未跟踪文件要在 add 之后再建,否则会被变成已 add 的新文件。
    std::fs::write(dir.path().join("modified.txt"), "v2\n").expect("write");
    std::fs::remove_file(dir.path().join("deleted.txt")).expect("remove");
    std::fs::rename(
        dir.path().join("renamed.txt"),
        dir.path().join("renamed-new.txt"),
    )
    .expect("rename");
    std::fs::write(dir.path().join("added.txt"), "added\n").expect("write");
    run_git(&["add", "-A"]);
    std::fs::write(dir.path().join("untracked.txt"), "new\n").expect("write");

    let statuses = DiffStateModel::file_statuses_against_head(dir.path())
        .await
        .expect("file_statuses_against_head");
    assert!(
        statuses.len() >= 5,
        "fixture 应产生至少 5 条改动,实际 {statuses:?}"
    );

    let batched = DiffStateModel::baselines_for_files(dir.path(), "HEAD", &statuses).await;
    assert_eq!(batched.len(), statuses.len());

    for (index, (path, status)) in statuses.iter().enumerate() {
        let oracle = DiffStateModel::get_file_content_at_head(dir.path(), path, status).await;
        assert_eq!(
            batched[index], oracle,
            "baseline 与逐个 git show 不一致: {path:?} {status:?}"
        );
    }
}

/// 回归:路径含换行时**不能**走按行分隔的批量协议 —— 请求会被拆成两个、多出的响应让后续
/// 请求整体错位。实测后果是该文件配到**别的文件的内容**(`we\nird.txt` 拿到 `we` 的内容)、
/// 其后的文件丢掉 baseline。这类路径必须退回逐文件 `git show`。
/// (Windows 不允许文件名含换行,故仅 unix。)
#[cfg(unix)]
#[tokio::test]
async fn baselines_for_files_falls_back_for_newline_paths() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let run_git = |args: &[&str]| {
        let output = command::blocking::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "test@test.com"]);
    run_git(&["config", "user.name", "Test"]);
    // "we" 与 "we\nird.txt" 的前缀关系,正是触发「配到别的文件内容」的条件。
    let files = [
        ("we", "I-AM-WE\n"),
        ("we\nird.txt", "I-AM-WEIRD\n"),
        ("after.txt", "I-AM-AFTER\n"),
    ];
    for (name, content) in files {
        std::fs::write(dir.path().join(name), content).expect("write fixture");
    }
    run_git(&["add", "-A"]);
    run_git(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", "init"]);

    // 改一遍内容,让三个文件都出现在 status 里(不改名、不新增)。
    for (name, _) in files {
        std::fs::write(dir.path().join(name), "v2\n").expect("write fixture");
    }

    let statuses = DiffStateModel::file_statuses_against_head(dir.path())
        .await
        .expect("file_statuses_against_head");
    assert_eq!(statuses.len(), 3, "fixture 应有 3 个改动文件: {statuses:?}");

    let batched = DiffStateModel::baselines_for_files(dir.path(), "HEAD", &statuses).await;

    for (index, (path, status)) in statuses.iter().enumerate() {
        let oracle = DiffStateModel::get_file_content_at_head(dir.path(), path, status).await;
        assert_eq!(batched[index], oracle, "baseline 不一致: {path:?}");
    }

    let newline_index = statuses
        .iter()
        .position(|(path, _)| path.to_string_lossy().contains('\n'))
        .expect("fixture 应包含含换行的路径");
    assert_eq!(
        batched[newline_index].as_deref(),
        Some("I-AM-WEIRD\n"),
        "含换行的路径必须取到自己的 baseline,不能串成 we 的内容"
    );
    // 排在它后面的文件也不能因为错位而丢掉 baseline。
    assert!(
        batched.iter().enumerate().any(
            |(index, value)| index != newline_index && value.as_deref() == Some("I-AM-AFTER\n")
        ),
        "换行路径之后的文件必须仍有 baseline: {batched:?}"
    );
}

/// 本地合成必须与 `git diff --no-index` 路径**整结构相同** —— 用 git 路径做 oracle。
/// 覆盖:普通文本 / 无尾换行 / 空文件 / 单空行 / 连续空行 / CRLF / 超长行 / 二进制。
#[tokio::test]
async fn synthesized_untracked_diff_matches_git_oracle() {
    let dir = tempfile::tempdir().expect("create temp dir");

    let long_line = "x".repeat(6000);
    let mut binary_bytes = vec![b'a'; 100];
    binary_bytes.push(0);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("plain.txt", b"one\ntwo\nthree\n".to_vec()),
        ("no_newline.txt", b"one\ntwo".to_vec()),
        ("empty.txt", Vec::new()),
        ("only_newline.txt", b"\n".to_vec()),
        ("blank_lines.txt", b"a\n\n\nb\n".to_vec()),
        ("crlf.txt", b"a\r\nb\r\n".to_vec()),
        ("long_line.txt", long_line.into_bytes()),
        ("binary.bin", binary_bytes),
    ];

    for (name, content) in cases {
        std::fs::write(dir.path().join(name), &content).expect("write fixture");

        let path = std::path::PathBuf::from(name);
        // New(已 add 的新文件)与 Untracked 在 git 路径里走同一个分支,合成也必须同时覆盖。
        for status in [GitFileStatus::Untracked, GitFileStatus::New] {
            let synthesized =
                DiffStateModel::synthesize_untracked_file_diff(dir.path(), &path, &status)
                    .expect("synthesis should succeed for a readable file");
            let oracle = DiffStateModel::get_file_diff(dir.path(), &path, &status, false, None)
                .await
                .expect("git oracle path should succeed");

            assert_eq!(
                synthesized, oracle,
                "本地合成与 git 路径不一致: {name} status={status:?}"
            );
        }
    }
}

#[tokio::test]
async fn untracked_directory_has_no_baseline_content() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::create_dir(repo_dir.path().join("nested-repo")).expect("create nested dir");
    std::fs::write(repo_dir.path().join("new-file.txt"), "hello\n").expect("write file");

    // 目录条目没有 baseline,因此不会为它构造 editor。
    let dir_content = DiffStateModel::get_file_content_at_head(
        repo_dir.path(),
        &std::path::PathBuf::from("nested-repo/"),
        &GitFileStatus::Untracked,
    )
    .await;
    assert_eq!(dir_content, None);

    // 普通 untracked 文件保留空 baseline。
    let file_content = DiffStateModel::get_file_content_at_head(
        repo_dir.path(),
        &std::path::PathBuf::from("new-file.txt"),
        &GitFileStatus::Untracked,
    )
    .await;
    assert_eq!(file_content, Some(String::new()));
}

#[tokio::test]
async fn num_lines_in_file_if_non_binary_counts_lines_in_text_file() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("file.txt");
    std::fs::write(&file_path, "one\ntwo\nthree\n").expect("write file");

    let num_lines = DiffStateModel::num_lines_in_file_if_non_binary(&file_path)
        .await
        .expect("counting a regular file should succeed");
    assert_eq!(num_lines, Some(3));
}

#[tokio::test]
async fn num_lines_in_file_if_non_binary_errors_for_directory() {
    let dir = tempfile::tempdir().expect("create temp dir");

    // 目录不可计数。metadata 调用方把该错误逐条目降级为 0 行贡献,
    // 而不是让整个 metadata 计算失败。
    let result = DiffStateModel::num_lines_in_file_if_non_binary(dir.path()).await;
    assert!(result.is_err());
}
