// 上游 cefsimple 的 lib.rs 仅为 Windows cdylib bootstrap 使用;本 spike 为
// macOS-only(见 Cargo.toml 注释与 specs/cef-webview-minimal/TECH.md 阶段 0),
// cdylib 目标保留以维持 helper 的链接形态,内容为空即无 mod:
