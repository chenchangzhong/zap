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
    /// `session_id` 为通知来源的 dsh 会话 id(插件侧按会话检测终态,基本必有;
    /// 旧版插件缺失时为 None),点击通知后用于在 dsh 内切到对应会话。
    Notify {
        title: String,
        body: String,
        category: NotificationCategory,
        session_id: Option<String>,
    },
    /// dsh 侧切换了当前项目目录(通知类,无回复)。
    SwitchProject { path: PathBuf },
    /// dsh 侧请求在 Zap 打开文件浏览器到指定项目目录。
    OpenFileExplorer { path: PathBuf },
    /// dsh 侧请求在 Zap 内打开一个文件/目录(文件链接拦截)。
    OpenFile { path: PathBuf },
    /// dsh 侧改动行点击:在 Zap 代码审核面板打开并定位到该文件。
    /// `path` 为 dsh 上报的绝对路径;改动卡片的表头按钮没有具体文件(打开
    /// 整个改动集),此时为 None。面板按 git 工作区 diff 定位,文件不在 diff
    /// 中时只打开面板(见 workspace 侧处理)。
    OpenCodeReview { path: Option<PathBuf> },
    /// runtime 就绪,`url` 为 dsh Web UI 地址。
    Ready { url: String },
    /// 崩溃后自动重启完成。
    Restarted { url: String },
    Failed { error: String },
}

/// 待主线程消费的事件(独立线程写入,主线程每帧 drain)。
static PENDING_EVENTS: LazyLock<Mutex<Vec<BridgeEvent>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// 将事件推入待处理队列(供 `drain_events` 在 `on_frame_drawn` 中消费)，
/// 并立即返回事件供调用方在同一主线程栈内 drain(确保 IPC 事件即时生效，
/// 不依赖下一帧绘制)。
pub(crate) fn push_event(event: BridgeEvent) -> BridgeEvent {
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
pub(crate) fn handle_zap_ipc(payload: &str) -> Option<BridgeEvent> {
    let mut parts = payload.splitn(3, '\n');
    let method = parts.next()?;
    let _id = parts.next(); // 通知类消息可忽略 id
    let params_json = parts.next().unwrap_or("{}");
    let params: Value =
        serde_json::from_str(params_json).unwrap_or(Value::Object(Default::default()));

    fn canonical_dir(raw: &str, method: &str) -> Option<PathBuf> {
        let path = PathBuf::from(raw);
        let canonical = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("[dsh-bridge] IPC {method} canonicalize failed: {e}");
                return None;
            }
        };
        if !canonical.is_dir() {
            log::warn!(
                "[dsh-bridge] IPC {method} path is not a directory: {}",
                canonical.display()
            );
            return None;
        }
        Some(canonical)
    }

    match method {
        "zap.switch_project" => {
            let raw = params.get("path")?.as_str()?;
            let canonical = canonical_dir(raw, "switch_project")?;
            log::info!("[dsh-bridge] IPC SwitchProject path={}", canonical.display());
            super::runtime::set_workspace_dir(canonical.clone());
            Some(push_event(BridgeEvent::SwitchProject { path: canonical }))
        }
        "zap.open_file_explorer" => {
            let raw = params.get("path")?.as_str()?;
            let canonical = canonical_dir(raw, "open_file_explorer")?;
            log::info!(
                "[dsh-bridge] IPC OpenFileExplorer path={}",
                canonical.display()
            );
            super::runtime::set_workspace_dir(canonical.clone());
            Some(push_event(BridgeEvent::OpenFileExplorer { path: canonical }))
        }
        "zap.open_file" => {
            let raw = params.get("path")?.as_str()?;
            let path = PathBuf::from(raw);
            // canonicalize 同时完成存在性校验(文件/目录均放行);不存在则忽略。
            let canonical = match path.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("[dsh-bridge] IPC {method} canonicalize failed: {e}");
                    return None;
                }
            };
            log::info!("[dsh-bridge] IPC OpenFile path={}", canonical.display());
            Some(push_event(BridgeEvent::OpenFile { path: canonical }))
        }
        "zap.open_code_review" => {
            // path 可选:改动卡片表头按钮只打开整个改动集,没有具体文件。
            // 文件行带绝对路径;放宽到允许不存在(目标可能已被删除/重命名),
            // 只做绝对路径归一,不做存在性校验,定位失败由 workspace 侧兜底。
            let path = match params.get("path") {
                None | Some(Value::Null) => None,
                Some(value) => {
                    let raw = value.as_str()?;
                    let path = PathBuf::from(raw);
                    if !path.is_absolute() {
                        log::warn!("[dsh-bridge] IPC {method} path is not absolute: {raw}");
                        return None;
                    }
                    Some(path)
                }
            };
            log::info!("[dsh-bridge] IPC OpenCodeReview path={path:?}");
            Some(push_event(BridgeEvent::OpenCodeReview { path }))
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
                session_id: params
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                    .map(Into::into),
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
