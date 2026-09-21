// 编译阶段 1 探针的 ObjC 宿主视图(probes/hole_probe.m)。
// 注意:本 crate 的 build script `rustc-link-lib` 实测未进入 bin 的链接行(cc 的 output 里有
// 该指令、rustc 行里只有 -L;framework=AppKit 同样未到),故 bin 侧用 `#[link]` 显式声明,
// 这里只负责编译静态库并提供 -L 搜索路径。
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("probes/hole_probe.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .compile("hole_probe_host");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rerun-if-changed=probes/hole_probe.m");
    }
}
