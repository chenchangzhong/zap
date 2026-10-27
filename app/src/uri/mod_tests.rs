//! `classify_open_file_action` 的路由测试。
//!
//! dsh 文件链接与 `file://` URL 都走 `uri::open_file`,分类错误会表现为"点图片却
//! 打开终端并尝试执行图片"(见 `classify_open_file_action` 的图片分支)。

use std::path::PathBuf;

use super::{classify_open_file_action, OpenFileAction};

fn write_temp_file(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("failed to write temp file");
    (dir, path)
}

#[test]
fn image_routes_to_editor() {
    // `is_file_openable_in_warp` 把 PNG 判成二进制 —— 图片必须靠扩展名分支进 Editor,
    // 否则会落到 ExecuteInSession(在父目录开终端并执行图片)。
    let (_dir, path) = write_temp_file("preview.png", b"\x89PNG\r\n\x1a\n");
    assert_eq!(classify_open_file_action(&path), OpenFileAction::Editor);
}

#[test]
fn svg_routes_to_editor() {
    let (_dir, path) = write_temp_file("icon.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
    assert_eq!(classify_open_file_action(&path), OpenFileAction::Editor);
}

#[test]
fn markdown_routes_to_notebook() {
    let (_dir, path) = write_temp_file("README.md", b"# Test\n");
    assert_eq!(classify_open_file_action(&path), OpenFileAction::Notebook);
}

#[test]
fn non_image_binary_opens_with_system_default() {
    // dmg / mp4 这类二进制既不能当文本打开,也不该落进终端执行。
    for name in ["installer.dmg", "clip.mp4"] {
        let (_dir, path) = write_temp_file(name, b"binary");
        assert_eq!(
            classify_open_file_action(&path),
            OpenFileAction::OpenWithSystemDefault,
            "{name} should be handed to the OS default application"
        );
    }
}

#[test]
fn unknown_extension_binary_content_opens_with_system_default() {
    // 白名单外的新格式:扩展名判不出来,靠文件头兜底。
    let (_dir, path) = write_temp_file("payload.bin2", &[0x00, 0x01, 0x02, 0xFF, 0x00]);
    assert_eq!(
        classify_open_file_action(&path),
        OpenFileAction::OpenWithSystemDefault
    );
}

#[test]
fn unknown_extension_text_content_stays_in_editor() {
    let (_dir, path) = write_temp_file("notes.bin2", b"hello world\n");
    assert_eq!(classify_open_file_action(&path), OpenFileAction::Editor);
}

#[test]
fn empty_unknown_extension_stays_in_editor() {
    // 空文件不能被内容嗅探误判成二进制(否则新建的空文件会交给系统程序)。
    let (_dir, path) = write_temp_file("empty.bin2", b"");
    assert_eq!(classify_open_file_action(&path), OpenFileAction::Editor);
}

#[test]
#[cfg(unix)]
fn executable_script_still_executes_in_session() {
    // 对照:可执行脚本仍按既有语义在终端里执行,不能被二进制分支误伤。
    use std::os::unix::fs::PermissionsExt;

    let (_dir, path) = write_temp_file("run.sh", b"#!/bin/sh\necho hi\n");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        classify_open_file_action(&path),
        OpenFileAction::ExecuteInSession
    );
}
