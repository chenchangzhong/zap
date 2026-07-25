# Lessons Learned

> 开发/构建中踩过的坑，下次注意。

## 1. `Harness` enum 改枚举时注意 exhaustive match 分支

在 `Harness`（`crates/warp_cli/src/agent.rs`）新增 variant 后，必须同步更新所有 match：

- `display_name()` — 显示名
- `Display` impl — 序列化名
- `parse_local_child_harness()` — 子 agent 白名单
- `brand_color()` / `icon_for()` / `display_name()` in `harness_display.rs`
- 所有 `match harness { ... }` 的调用处

编译器会报 `non-exhaustive patterns`，但容易看漏。

## 2. `harness_kind()` 改 match 时别把其他分支吃了

用 edit 工具 SWAP 行范围时，如果范围覆盖了其他分支（Claude、OpenCode），payload 里没列它们就会消失。  
**教训**：每次编辑后跑 `cargo check` 确认。

## 3. HarnessSelector 创建了 ViewHandle 但没加入渲染树

`input.rs` 里 `HarnessSelector::new()` 注册了 View，但 `render_agent_input()` 没把 `ChildView::new(&self.harness_selector)` 加上。所以 UI 上一直看不到。  
**教训**：创建 View 后检查是否真的渲染了。

## 4. RPC 消息走了 bracketed paste 路径

`send_text_to_cli()` → `submit_text_to_cli_agent_pty()` 对 `CLIAgent::OhMyPi` 走 `RichInputSubmitStrategy::BracketedPaste`，给 RPC JSON 加了 `\x1b[200~...\x1b[201~\r`。OMP 的 RPC 模式读原始 NDJSON，被包浆了。  
**教训**：结构化协议（RPC/NDJSON）必须用 `write_to_pty()` 直写，别走 agent text submission 管线。

## 5. `parse_local_child_harness` 和 `prepare_local_harness_child_launch` 不一致

`parse_local_child_harness` 接受了 `OhMyPi`，但 `prepare_local_harness_child_launch` 里 `Harness::OhMyPi => unreachable!()`。因为 `normalize_local_child_harness` 直接委托给 `parse_local_child_harness`，运行时会直接 panic。  
**教训**：`parse_local_child_harness` 的接受分支和 `prepare_local_harness_child_launch` 的 `unreachable!` 必须对齐。新增 enum variant 时两个方向都要检查。

## 6. 模型 ID 带 UUID 前缀

`into_agent_provider_model()` 直接把 models.dev 的 `model.id` 写入本地配置，其中包含 `{provider-uuid}/{model-name}` 的 UUID 前缀。这会导致：
- 设置页显示 UUID+模型名（用户不可读）
- 传给上游 API 时 model 参数带无效前缀

**教训**：外部数据进入本地配置前要清洗格式。

## 7. `send_raw_to_pty` — 新方法签名确认

加 `TerminalDriver::send_raw_to_pty()` 时用了 `&str` 参数。TerminalView 的 `write_to_pty` 接受 `B: Into<Cow<'static, [u8]>>`，把 `data.as_bytes().to_vec()` 传进去是 OK 的。

## 8. macOS 27 红绿灯修复

Commit `7edcf87a0` 修了 macOS 27 下红绿灯点击，但后来 `10d9ad3a1` 回退了这组改动（破坏了其他版本的原生窗口事件）。  
**教训**：要用 `@available(macOS 27, *)` 条件编译，不能在旧版本上执行只能用新 API 的路径。  
**当前 fix**：坐标转换 + `performClose:` 只对 27+ 生效，<27 保持旧的 `convertPoint:` + `mouseDown:`。

## 9. 构建时缺 feature 标记

- 主题跟随系统：`system_theme` feature 在 `Cargo.toml` 有定义但代码里没实现，加上也没用。
- 构建时 `cargo bundle --features "..."` 容易漏 feature。对照 `script/macos/run` 里的 feature 列表检查。

## 10. `compile_icon` 传错 channel 参数

`script/compile_icon` 需要传正确的 channel（如 `oss`），而 `script/macos/run` 传的是 `$WARP_SCHEME_NAME`（`zap`）。导致 `channels/zap/icon/AppIcon.icon` 找不到，跳过了编译。  
**后果**：没有 `Assets.car`（自适应图标容器），app 图标不会跟随系统外观。  
**教训**：手动构建时用 `./script/compile_icon oss Path/To/App.app`。

## 11. `over-air macOS 27` `convertPoint:fromView:nil` 坐标错误

macOS 27 上 `[button convertPoint:event.locationInWindow fromView:nil]` 对 titlebar buttons 返回错误坐标，导致 `NSPointInRect` 永远不命中，`standardWindowButtonAtEvent:` 返回 `nil`，红绿灯点击无响应。  
**修复**：手动计算 `windowHeight - titlebarHeight + button.frame.origin.y`。

## 12. 编译加速

|措施|效果|
|---|---|
|`sccache`|缓存编译产物，全量重编 2m06s → 55s|
|`ld64.lld` 链接器|增量编译 ~26s（~600MB 二进制链接瓶颈）|
|不删 `target/debug/deps`|避免全量重编依赖|

## 13. 合并冲突在测试文件中

`app/src/ai/agent/task_store_tests.rs` 有残留的 `>>>>>>>` 合并冲突标记，导致 `cargo test` build 失败（非首次遇到）。  
**教训**：merge 后检查冲突标记。有问题的文件在 `rg '<<<<<<|======|>>>>>>' --type rust`。
