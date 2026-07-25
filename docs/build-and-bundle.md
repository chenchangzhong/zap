# 构建与打包指南

## 前置条件

- Rust 工具链（`rustup`）
- Xcode + Command Line Tools
- 签名证书（Apple Development / Distribution）

## 快速构建（debug 运行）

```bash
cargo run -p warp
```

## 发布版打包

### 1. 清理旧缓存（可选）

```bash
cargo clean
```
清理约 21.6GB 编译缓存。构建时长约 6–8 分钟。

### 2. 构建 DockTilePlugin

DockTilePlugin 须在打 .app bundle 前编译到 `target/` 下。

```bash
make -C app/DockTilePlugin clean
make -C app/DockTilePlugin
cp -R app/DockTilePlugin/ZapDockTilePlugin.docktileplugin target/release-lto/
```

注意：构建前确保 `target/release-lto/ZapDockTilePlugin.docktileplugin` 不存在，否则 bundle 脚本会报错退出。

### 3. 构建 .app bundle

```bash
./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64
```

参数说明：
- `--channel oss` — 开源版渠道
- `--selfsign` — 使用本地 Apple Development 证书签名
- `--nouniversal --arch aarch64` — 仅编译 ARM64（Apple Silicon），不编译 x86_64

产出的 `.app` 路径：
```
target/aarch64-apple-darwin/release-lto/bundle/osx/Zap.app
```

### 4. 编译自适应图标

macOS Sequoia 支持根据系统主题（浅色/深色）自动切换图标。需要用 `actool` 编译 `.icon` bundle：

```bash
script/compile_icon oss target/aarch64-apple-darwin/release-lto/bundle/osx/Zap.app
```

该步骤产生：
- `AppIcon.icns` — 后备图标
- `Assets.car` — 自适应图标资源（系统主题切换用）

`.icon` bundle 源文件位置：
```
app/channels/oss/icon/AppIcon.icon/
```

若该目录不存在，`compile_icon` 会静默跳过（仅 OSS 渠道），导致 `Assets.car` 缺失。

### 5. 重新签名

编译图标后 bundle 内容发生了变化，**必须重新签名**，否则 macOS 会拒绝运行或丢失权限。

```bash
codesign --force --deep --options runtime \
  --sign "CF74906DE6F1B22ACEE43D6BD020A6589F6FC161" \
  --entitlements script/Debug-Entitlements.plist \
  target/aarch64-apple-darwin/release-lto/bundle/osx/Zap.app
```

验证签名：
```bash
codesign -dv target/aarch64-apple-darwin/release-lto/bundle/osx/Zap.app
```

正常输出应包含 `Signature size=xxxx`（不是 `adhoc`）。

### 6. 打包 DMG

```bash
BUNDLE=target/aarch64-apple-darwin/release-lto/bundle
rm -f "$BUNDLE/dmg/Zap.dmg"
hdiutil create -volname "Zap" \
  -srcfolder "$BUNDLE/osx/Zap.app" \
  -ov -format UDZO \
  "$BUNDLE/dmg/Zap.dmg"
```

## 常见问题

### `--skip-build` 的问题

`--skip-build` 会导致：
- 跳过 `compile_icon`，产出的 .app 没有自适应图标
- 路径错位，.app 会跑到 `dmg/Zap.app` 而非 `osx/Zap.app`
- 只使用 `--skip-build` 然后手动补 `compile_icon`，仍需重新签名和重建 DMG

**建议**：除非调试打包流程，否则不要使用 `--skip-build`。

### `--selfsign` 签名不生效

`--selfsign` 有时会回退到 ad-hoc 签名而非开发证书。表现：`codesign -dv` 输出 `Signature=adhoc`。

**原因**：bundle 脚本的签名逻辑依赖 `security find-identity | grep "Apple Development"`，可能找不到或选错证书。

**解决**：`--selfsign` 完后手动用证书哈希签名（见第 5 步）。

### `ZapDockTilePlugin.docktileplugin 找不到`

bundle 脚本期望 `target/release-lto/ZapDockTilePlugin.docktileplugin` 已存在。如果执行过 `cargo clean`，这个产物会被清除，需要重新构建（见第 2 步）。

### `DockTilePlugin` 的 .gitignore

`app/DockTilePlugin/ZapDockTilePlugin.docktileplugin/` 是编译产物，需加入 `.gitignore`，避免误提交。

### 图标目录不存在

`app/channels/oss/icon/AppIcon.icon/` 必须存在，否则 `compile_icon` 静默跳过，不生成 `Assets.car`。
