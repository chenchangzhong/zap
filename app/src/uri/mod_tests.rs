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
fn non_image_binary_still_executes_in_session() {
    // 对照:不能被图片分支误伤 —— MP4 仍按既有策略在终端里执行。
    let (_dir, path) = write_temp_file("clip.mp4", b"\x00\x00\x00\x18ftypmp42");
    assert_eq!(
        classify_open_file_action(&path),
        OpenFileAction::ExecuteInSession
    );
}
