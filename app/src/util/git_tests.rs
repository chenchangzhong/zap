use std::path::Path;

use command::r#async::Command;
use command::Stdio;
use tempfile::TempDir;

use super::{detect_current_branch, detect_current_branch_display};

/// Helper: run a git command inside the given repo directory.
async fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("failed to run git");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Creates a temp git repo with one commit and returns `(dir_handle, repo_path)`.
async fn init_repo() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().to_path_buf();

    git(&path, &["init", "-b", "main"]).await;
    git(&path, &["config", "user.email", "test@test.com"]).await;
    git(&path, &["config", "user.name", "Test"]).await;
    git(&path, &["commit", "--allow-empty", "-m", "initial"]).await;

    (dir, path)
}

#[tokio::test]
async fn on_normal_branch_returns_branch_name() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "-b", "feature-xyz"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "feature-xyz");
    assert_eq!(
        detect_current_branch_display(&repo).await.unwrap(),
        "feature-xyz"
    );
}

#[tokio::test]
async fn detached_head_raw_returns_head() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "HEAD");
}

#[tokio::test]
async fn detached_head_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(
        result, "HEAD",
        "display variant should not return literal HEAD"
    );
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

#[tokio::test]
async fn detached_tag_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["tag", "v1.0"]).await;
    git(&repo, &["checkout", "v1.0"]).await;

    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(result, "HEAD");
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

/// 正常路径:短命令在超时窗口内返回输出。
#[cfg(all(feature = "local_fs", unix))]
#[test]
fn run_command_with_timeout_returns_output() {
    let mut cmd = command::blocking::Command::new("echo");
    cmd.arg("hi")
        .stdout(command::Stdio::piped())
        .stderr(command::Stdio::piped());

    let (status, stdout, _stderr) =
        super::run_command_with_timeout(&mut cmd, Some(std::time::Duration::from_secs(5)), None)
            .expect("短命令应成功");

    assert!(status.success());
    assert_eq!(String::from_utf8_lossy(&stdout).trim(), "hi");
}

/// `timeout = None`(网络类命令 push/fetch)不做超时:命令照常返回。
#[cfg(all(feature = "local_fs", unix))]
#[test]
fn run_command_without_timeout_returns_output() {
    let mut cmd = command::blocking::Command::new("echo");
    cmd.arg("no-timeout")
        .stdout(command::Stdio::piped())
        .stderr(command::Stdio::piped());

    let (status, stdout, _stderr) =
        super::run_command_with_timeout(&mut cmd, None, None).expect("应成功");

    assert!(status.success());
    assert_eq!(String::from_utf8_lossy(&stdout).trim(), "no-timeout");
}

/// 超时兜底:命令超时必须被 kill 并返回 Err,绝不能永久挂住。
///
/// 背景见 `GIT_LOCAL_COMMAND_TIMEOUT` 注释:`command::async`(async-process)会丢失
/// 快速退出子进程的完成事件,导致 `output().await` 永久挂起(审核面板卡 Loading)。
#[cfg(all(feature = "local_fs", unix))]
#[test]
fn run_command_with_timeout_kills_long_running_command() {
    let started = std::time::Instant::now();
    let mut cmd = command::blocking::Command::new("sleep");
    cmd.arg("30")
        .stdout(command::Stdio::piped())
        .stderr(command::Stdio::piped());

    let result = super::run_command_with_timeout(
        &mut cmd,
        Some(std::time::Duration::from_millis(200)),
        None,
    );

    assert!(result.is_err(), "超时应返回 Err");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "超时后应很快返回,不能挂住"
    );
}

/// stdin 通道:载荷远超管道缓冲(64KB)时也必须完整送达且不死锁 ——
/// 这正是 `cat-file --batch` 的用法(请求列表经 stdin 传入)。
/// 若把「写 stdin」放在启动 stdout/stderr 读取线程之前,这个测试会挂住。
#[cfg(all(feature = "local_fs", unix))]
#[test]
fn run_command_with_timeout_delivers_large_stdin_without_deadlock() {
    let payload = "x".repeat(1024 * 1024);

    let mut cmd = command::blocking::Command::new("cat");
    cmd.stdout(command::Stdio::piped())
        .stderr(command::Stdio::piped());

    let (status, stdout, _stderr) = super::run_command_with_timeout(
        &mut cmd,
        Some(std::time::Duration::from_secs(10)),
        Some(payload.clone().into_bytes()),
    )
    .expect("cat 应成功");

    assert!(status.success());
    assert_eq!(String::from_utf8_lossy(&stdout), payload);
}
