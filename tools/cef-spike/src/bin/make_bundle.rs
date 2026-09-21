// 组 .app bundle 的薄壳:复用 tauri-apps/cef-rs 的 build_util(mac arm64 途径),
// 行为对齐官方 `bundle-cef-app`:从 cwd 的 cargo metadata 读
// [package.metadata.cef.bundle],cargo build 主/ helper 两 bin,再嵌 CEF framework
// 与 5 个 helper 子 app(specs/cef-webview-minimal/TECH.md 阶段 0 验收 b)。
use cef::build_util::mac::{build_bundle, BundleInfo};
use semver::Version;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let name = std::env::args()
        .nth(1)
        .expect("usage: make-bundle <name> [-o output-dir]");
    let mut output = None;
    let mut it = std::env::args().skip(2);
    while let Some(arg) = it.next() {
        if arg == "-o" || arg == "--output" {
            output = it.next().map(PathBuf::from);
        }
    }
    let output = output.unwrap_or_else(|| std::env::current_dir().unwrap());
    let app_path = build_bundle(
        &output,
        &name,
        BundleInfo {
            name: name.clone(),
            identifier: format!("dev.zap.cef-spike.{name}"),
            display_name: name.clone(),
            development_region: "English".to_owned(),
            version: Version::new(0, 1, 0),
        },
    )?;
    println!("bundle ready: {}", app_path.display());
    Ok(())
}
