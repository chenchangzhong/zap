//! DshBridge:Zap 侧 IPC 桥(浏览器端插件经 webview IPC 与 Zap 通信)。
//!
//! 职责:
//! - 接收浏览器端插件(`zap-bridge-client.js`)经 `webkit.messageHandlers.ipc`
//!   发来的 `zap.*` 消息(项目切换、打开 code review 等)
//! - 事件经全局暂存缓冲,由主线程每帧 `drain_events` → `ModelContext::emit`
//!
//! 通信方式:webview IPC(不再使用 WebSocket 桥)。

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::LazyLock;

use serde_json::Value;
use warpui::ModelContext;

use crate::notifications::item::NotificationCategory;

/// 桥对外事件(主线程消费,驱动面板 UI 连接状态)。
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// dsh 插件请求发送通知(任务完成/出错/需确认)。
    Notify {
        title: String,
        body: String,
        category: NotificationCategory,
    },
    /// dsh 侧切换了当前项目目录(通知类,无回复)。
    SwitchProject { path: PathBuf },
    /// runtime 就绪,`url` 为 dsh Web UI 地址。
    Ready { url: String },
    /// 检测到 dsh 新版本,正在更新。
    Updating { version: String },
    /// 崩溃后自动重启完成。
    Restarted { url: String },
    /// 启动/重启失败。
    Failed { error: String },
}

/// 待主线程消费的事件(独立线程写入,主线程每帧 drain)。
static PENDING_EVENTS: LazyLock<Mutex<Vec<BridgeEvent>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// 将事件推入待处理队列(供 `drain_events` 在 `on_frame_drawn` 中消费)，
/// 并立即返回事件供调用方在同一主线程栈内 drain(确保 IPC 事件即时生效，
/// 不依赖下一帧绘制)。
fn push_event(event: BridgeEvent) -> BridgeEvent {
    PENDING_EVENTS.lock().push(event.clone());
    event
}

/// 检查是否有待处理事件(供 on_frame_drawn fallback 检测残余)。
pub fn has_pending_events() -> bool {
    !PENDING_EVENTS.lock().is_empty()
}

pub fn drain_events(ctx: &mut ModelContext<super::DshRuntime>) {
    let events: Vec<BridgeEvent> = PENDING_EVENTS.lock().drain(..).collect();
    for event in events {
        ctx.emit(event);
    }
}

/// 处理浏览器端插件经 webview IPC 发来的 zap.* 消息。
/// payload 格式: `"method\nid\nparams_json"`(由 zap-bridge-client.js 构造)。
/// 返回事件供调用方立即 drain(确保 IPC 事件即时生效，不依赖下一帧绘制)。
pub(crate) fn handle_zap_ipc(payload: &str) -> Option<BridgeEvent> {
    let mut parts = payload.splitn(3, '\n');
    let method = parts.next()?;
    let _id = parts.next(); // 通知类消息可忽略 id
    let params_json = parts.next().unwrap_or("{}");
    let params: Value =
        serde_json::from_str(params_json).unwrap_or(Value::Object(Default::default()));

    match method {
        "zap.switch_project" => {
            let path = params.get("path").and_then(|p| p.as_str()).map(PathBuf::from)?;
            // 路径校验:canonicalize + 存在性 + 目录检查。
            let canonical = match path.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("[dsh-bridge] IPC switch_project canonicalize failed: {e}");
                    return None;
                }
            };
            if !canonical.is_dir() {
                log::warn!(
                    "[dsh-bridge] IPC switch_project path is not a directory: {}",
                    canonical.display()
                );
                return None;
            }
            log::info!(
                "[dsh-bridge] IPC SwitchProject path={}",
                canonical.display()
            );
            super::runtime::set_workspace_dir(canonical.clone());
            Some(push_event(BridgeEvent::SwitchProject { path: canonical }))
        }
        "zap.notify" => {
            let title = params
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let body = params
                .get("body")
                .and_then(|b| b.as_str())
                .unwrap_or("")
                .to_string();
            if title.is_empty() {
                return None;
            }
            let category = match params.get("category").and_then(|c| c.as_str()) {
                Some("error") => NotificationCategory::Error,
                Some("confirm") => NotificationCategory::Request,
                _ => NotificationCategory::Complete,
            };
            Some(push_event(BridgeEvent::Notify {
                title,
                body,
                category,
            }))
        }
        _ => {
            log::debug!("[dsh-bridge] IPC unknown method: {method}");
            None
        }
    }
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
