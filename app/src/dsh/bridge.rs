//! DshBridge:Zap 侧 WebSocket 桥服务(阶段 0)。
//!
//! 职责:
//! - 在 `127.0.0.1:<随机端口>` 起 WebSocket server(`async_tungstenite`)
//! - 生成并持有握手 token(每次启动随机,防本地进程越权)
//! - 握手校验:`bridge/hello` 携带 token + dsh 版本/能力 → 校验 → `Connected`
//! - 分发 JSON-RPC 2.0 请求(阶段 0:`bridge/hello`、`zap.ping`)
//! - 事件经全局暂存缓冲,由主线程每帧 `drain_events` → `ModelContext::emit`
//!
//! 并发模型:独立 tokio runtime 跑 accept loop + 每连接消息循环(借用
//! `HttpServer` / `BrowserWebViewManager` 模式);`port`/`token` 在 `new()` 同步
//! 生成,供 `runtime.rs` 写 cordis.yml 注入 dsh。

use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures_util::StreamExt as _;
use ignore::WalkBuilder;
use serde_json::{json, Value};
use tokio_util::compat::TokioAsyncReadCompatExt;
use warpui::{Entity, ModelContext, SingletonEntity};

/// 协议版本(递增;不匹配时握手明确报错)。
const PROTOCOL_VERSION: u32 = 1;
/// Zap 提供的能力清单(握手响应返回,dsh 据此决定注册哪些 `zap_*` 工具)。
/// 与 `handle_zap_method` / `handle_message` 实际实现的方法一致。
const ZAP_CAPABILITIES: &[&str] = &[
    "files.list",
    "files.read",
    "files.search",
    "terminal.context",
];
/// 握手超时:连接后未在此时长内完成握手即关闭(防半开连接)。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// token 字符数。
const TOKEN_LEN: usize = 32;
/// `zap.read_file` 最大读入字节(防超大文件吃内存)。
const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

/// JSON-RPC 2.0 错误码 + 桥应用错误码。
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// token 无效 / 未认证。
    pub const UNAUTHORIZED: i64 = 1001;
    /// 协议版本不匹配。
    pub const PROTOCOL_MISMATCH: i64 = 1002;
    // ── zap.* 业务错误(1000x) ──
    /// 路径不是目录。
    pub const NOT_A_DIR: i64 = 10010;
    /// 读目录失败。
    pub const READ_DIR: i64 = 10011;
    /// 读文件失败。
    pub const READ_FILE: i64 = 10012;
    /// 路径非法(绝对路径 / 逃逸项目根 / 解析失败)。
    pub const PATH_INVALID: i64 = 10013;
    /// 文件超过读取上限。
    pub const FILE_TOO_LARGE: i64 = 10014;
}

/// 桥对外状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeState {
    Down,
    Listening,
    Ready,
    HandshakeFailed,
}

/// 桥对外事件(主线程消费,驱动面板 UI 连接状态)。
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// server 就绪,`port` 为监听端口。
    Listening { port: u16 },
    /// 握手成功,连接建立。
    Connected {
        dsh_version: String,
        capabilities: Vec<String>,
    },
    /// 连接断开。
    Disconnected,
    /// 握手失败(原因:token 无效 / 版本不匹配 / 超时)。
    HandshakeFailed { reason: String },
}

/// 待主线程消费的事件(独立 tokio runtime 线程写入,主线程每帧 drain)。
static PENDING_EVENTS: LazyLock<Mutex<Vec<BridgeEvent>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

fn push_event(event: BridgeEvent) {
    PENDING_EVENTS.lock().push(event);
}

/// 桥启动信息(port + token),供 `runtime.rs` 生成 cordis.yml 注入 dsh。
/// `BridgeServer::new()` 写入;`start_inner`(异步关联函数,不借用 model)
/// 读取。未启动时为 `None`。
static BRIDGE_INFO: LazyLock<Mutex<Option<(u16, String)>>> =
    LazyLock::new(|| Mutex::new(None));

/// 读取桥启动信息(port + token)。桥未启动时为 `None`。
pub(crate) fn bridge_info() -> Option<(u16, String)> {
    BRIDGE_INFO.lock().clone()
}

/// Zap 侧桥服务单例。生命周期挂到 DshRuntime 启停:懒启动,首次打开 dsh
/// pane 时由 `open_dsh_pane` 调用 [`BridgeServer::start`] 拉起 listener;
/// dsh 停止时由 pane detach 调用 [`BridgeServer::stop`] 关停并清空
/// `BRIDGE_INFO`,避免旧 token 的桥在 dsh 停止后仍被同机进程调用。桥关闭
/// 时 listener 由 `Drop` 兜底关停。
pub struct BridgeServer {
    runtime: Option<tokio::runtime::Runtime>,
    port: u16,
    token: Arc<str>,
    state: BridgeState,
}

impl Default for BridgeServer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BridgeServer {
    fn drop(&mut self) {
        // accept loop 是无限任务,用 shutdown_background 立即返回,避免阻塞。
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
        *BRIDGE_INFO.lock() = None;
    }
}

impl BridgeServer {
    /// 懒创建:不启动 listener。首次打开 dsh pane 时由 `start()` 拉起。
    pub fn new() -> Self {
        Self {
            runtime: None,
            port: 0,
            token: Arc::from(""),
            state: BridgeState::Down,
        }
    }

    /// 启动桥(幂等):已运行则保持;否则新建 listener 与随机 token,并把
    /// (port, token) 写入 `BRIDGE_INFO` 供 `start_inner` 注入 dsh。
    pub fn start(&mut self) {
        if self.runtime.is_some() {
            return;
        }
        let token: Arc<str> = Arc::from(generate_token());
        let (runtime, port) = match Self::spawn_listener(token.clone()) {
            Ok(rt) => rt,
            Err(err) => {
                log::error!("[dsh-bridge] failed to start: {err:#}");
                self.state = BridgeState::Down;
                return;
            }
        };
        *BRIDGE_INFO.lock() = Some((port, token.to_string()));
        push_event(BridgeEvent::Listening { port });
        self.runtime = Some(runtime);
        self.port = port;
        self.token = token;
        self.state = BridgeState::Listening;
    }

    /// 停止桥:关停 listener runtime、清空 `BRIDGE_INFO` 与待处理事件,状态
    /// 置 `Down`。幂等;停止后调用 `start()` 可重新拉起(带新 token)。
    pub fn stop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
        PENDING_EVENTS.lock().clear();
        *BRIDGE_INFO.lock() = None;
        self.port = 0;
        self.token = Arc::from("");
        self.state = BridgeState::Down;
    }

    /// 同步 bind 随机端口(得 `port` 供 cordis.yml),在独立 tokio runtime
    /// 上跑 accept loop。返回 (runtime, port)。
    fn spawn_listener(token: Arc<str>) -> Result<(tokio::runtime::Runtime, u16), std::io::Error> {
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = std_listener.local_addr()?.port();
        std_listener.set_nonblocking(true)?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .enable_time()
            .build()?;
        // `TcpListener::from_std` 需要 tokio reactor,而 `BridgeServer::new()`
        // 在 app 主线程(非 tokio runtime 上下文)调用,故转 tokio listener
        // 必须在独立 runtime 内执行(否则 panic "no reactor running")。
        runtime.spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(std_listener) {
                Ok(listener) => listener,
                Err(err) => {
                    log::error!("[dsh-bridge] from_std failed: {err:#}");
                    return;
                }
            };
            accept_loop(listener, token).await;
        });
        Ok((runtime, port))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn state(&self) -> BridgeState {
        self.state
    }

    /// 主线程每帧消费待处理事件,经 model emit 分发,并同步内部状态。
    pub fn drain_events(&mut self, ctx: &mut ModelContext<Self>) {
        let events: Vec<BridgeEvent> = PENDING_EVENTS.lock().drain(..).collect();
        for event in events {
            match &event {
                BridgeEvent::Connected { .. } => self.state = BridgeState::Ready,
                BridgeEvent::Disconnected => self.state = BridgeState::Listening,
                BridgeEvent::HandshakeFailed { .. } => self.state = BridgeState::HandshakeFailed,
                BridgeEvent::Listening { .. } => {}
            }
            ctx.emit(event);
        }
    }
}

impl Entity for BridgeServer {
    type Event = BridgeEvent;
}

impl SingletonEntity for BridgeServer {}

/// 生成随机 token(字母数字,`TOKEN_LEN` 位)。
fn generate_token() -> String {
    use rand::distributions::Alphanumeric;
    use rand::Rng;
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(TOKEN_LEN)
        .map(char::from)
        .collect()
}

/// accept loop:持续接受连接,每连接起一个消息循环任务。
async fn accept_loop(listener: tokio::net::TcpListener, token: Arc<str>) {
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(s) => s,
            Err(err) => {
                log::error!("[dsh-bridge] accept error: {err}");
                // 短暂退避,避免持续 accept 错误时忙循环。
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        tokio::spawn(handle_connection(stream, token.clone()));
    }
}

/// 单连接处理:WS 握手 → 消息循环(握手校验 + JSON-RPC 分发)。
async fn handle_connection(stream: tokio::net::TcpStream, token: Arc<str>) {
    // 限制最大消息/帧大小(1 MiB),防恶意客户端超大帧吃内存。
    let config = async_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(1024 * 1024),
        max_frame_size: Some(1024 * 1024),
        ..Default::default()
    };
    let mut ws =
        match async_tungstenite::accept_async_with_config(stream.compat(), Some(config)).await {
            Ok(ws) => ws,
            Err(err) => {
                log::debug!("[dsh-bridge] ws accept failed: {err}");
                return;
            }
        };

    let mut handshaken = false;
    // 握手失败后 Disconnected 不再上抛,保留 HandshakeFailed 状态。
    let mut failed = false;
    // 握手超时仅在未完成握手阶段生效;握手成功后读无超时(长连接)。
    let handshake_deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;

    loop {
        let read = if handshaken {
            ws.next().await
        } else {
            match tokio::time::timeout_at(handshake_deadline, ws.next()).await {
                Ok(r) => r,
                Err(_) => {
                    push_event(BridgeEvent::HandshakeFailed {
                        reason: "handshake timeout".into(),
                    });
                    failed = true;
                    break;
                }
            }
        };
        let msg = match read {
            Some(Ok(msg)) => msg,
            Some(Err(err)) => {
                log::debug!("[dsh-bridge] read error: {err}");
                break;
            }
            // 连接关闭。
            None => break,
        };

        let text = match msg.into_text() {
            Ok(t) => t,
            Err(_) => {
                // 非文本帧(如二进制):忽略或关闭。
                continue;
            }
        };

        // zap.* 文件能力方法:需已握手,根为当前 Zap 项目目录;同步文件 IO
        // 移入 spawn_blocking,避免阻塞单 worker runtime(大仓库搜索 / 慢
        // IO 会冻结 accept 循环与其它连接)。zap.ping / zap.terminal_context
        // 不需项目根,回落 handle_message 统一分发。
        if handshaken {
            if let Some((zap_method, zap_params, zap_id)) = parse_rpc_head(&text) {
                if matches!(
                    zap_method.as_str(),
                    "zap.list_files" | "zap.read_file" | "zap.search"
                ) {
                    // 通知(无 id):文件方法只读无副作用,无需执行;且 JSON-RPC
                    // 规定通知不得有响应,跳过避免发 `id: null` 的垃圾帧。
                    if zap_id.is_null() {
                        continue;
                    }
                    let result: Result<Value, RpcError> =
                        match super::runtime::workspace_dir() {
                            Some(root) => {
                                let method = zap_method.clone();
                                let params = zap_params.clone();
                                tokio::task::spawn_blocking(move || {
                                    handle_zap_method(&method, &params, &root)
                                })
                                .await
                                .unwrap_or_else(|join_err| Err(RpcError {
                                    code: code::INTERNAL_ERROR,
                                    message: format!("file op panicked: {join_err}"),
                                    id: Some(zap_id.clone()),
                                }))
                            }
                            None => Err(RpcError {
                                code: code::INVALID_REQUEST,
                                message: "no Zap workspace (project) set".into(),
                                id: Some(zap_id.clone()),
                            }),
                        };
                    match result {
                        Ok(value) => {
                            let resp = json!({ "jsonrpc": "2.0", "id": zap_id, "result": value });
                            if ws
                                .send(async_tungstenite::tungstenite::Message::Text(
                                    resp.to_string(),
                                ))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(err) => {
                            // 始终用请求 id 发错误响应:handle_zap_method 内部
                            // 错误 id 可能为 None,否则 dsh 侧 rpc promise 永不
                            // resolve,工具调用卡住(真实验收发现)。
                            let resp = json!({
                                "jsonrpc": "2.0",
                                "id": zap_id,
                                "error": { "code": err.code, "message": err.message },
                            });
                            let _ = ws
                                .send(async_tungstenite::tungstenite::Message::Text(
                                    resp.to_string(),
                                ))
                                .await;
                        }
                    }
                    continue;
                }
            }
        }

        match handle_message(&token, handshaken, &text) {
            Ok(outcome) => {
                for event in outcome.events {
                    if matches!(event, BridgeEvent::Connected { .. }) {
                        handshaken = true;
                    }
                    push_event(event);
                }
                if let Some(resp) = outcome.response {
                    if ws
                        .send(async_tungstenite::tungstenite::Message::Text(resp.to_string()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Err(err) => {
                // 发错误响应(若有 id)。
                if let Some(id) = &err.id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": err.code, "message": err.message },
                    });
                    let _ = ws
                        .send(async_tungstenite::tungstenite::Message::Text(resp.to_string()))
                        .await;
                }
                // 仅未握手阶段的错误(畸形请求 / token 错 / 版本不匹配)才判定
                // 握手失败并关闭;已握手长连接上的普通 JSON-RPC 错误只回 error
                // 响应,连接保持(避免一条坏消息杀死健康连接)。
                if !handshaken {
                    push_event(BridgeEvent::HandshakeFailed {
                        reason: err.message,
                    });
                    failed = true;
                    break;
                }
            }
        }
    }

    // 握手失败已上抛 HandshakeFailed,不再补 Disconnected(避免覆盖其状态)。
    if !failed {
        push_event(BridgeEvent::Disconnected);
    }
}

/// 单条 JSON-RPC 消息的纯处理逻辑(无副作用,可单测)。
///
/// 返回响应(通知为 `None`)与要上抛的事件;错误返回 [`RpcError`]。
fn handle_message(
    token: &str,
    handshaken: bool,
    raw: &str,
) -> Result<MessageOutcome, RpcError> {
    // 1. 解析。
    let msg: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => {
            return Err(RpcError {
                code: code::PARSE_ERROR,
                message: "parse error".into(),
                id: None,
            })
        }
    };
    let obj = match msg.as_object() {
        Some(o) => o,
        None => {
            return Err(RpcError {
                code: code::INVALID_REQUEST,
                message: "request must be a JSON object".into(),
                id: None,
            })
        }
    };
    let id = obj.get("id").cloned();
    let method = match obj.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => {
            return Err(RpcError {
                code: code::INVALID_REQUEST,
                message: "missing method".into(),
                id,
            })
        }
    };
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));

    // 2. 分发(错误统一补请求 id,便于错误响应关联)。
    let outcome = match method {
        "bridge/hello" => handle_hello(token, &params),
        "zap.ping" => {
            if !handshaken {
                Err(RpcError {
                    code: code::UNAUTHORIZED,
                    message: "not authenticated (send bridge/hello first)".into(),
                    id: None,
                })
            } else {
                Ok(MessageOutcome {
                    response: Some(json!({ "ok": true })),
                    events: Vec::new(),
                })
            }
        }
        // 不需项目根:不经过 fast path(handle_connection 只拦截文件方法),
        // 在 handle_message 统一处理,避免错误依赖 workspace_dir 已设置。
        "zap.terminal_context" => {
            if !handshaken {
                Err(RpcError {
                    code: code::UNAUTHORIZED,
                    message: "not authenticated (send bridge/hello first)".into(),
                    id: None,
                })
            } else {
                zap_terminal_context().map(|result| MessageOutcome {
                    response: Some(result),
                    events: Vec::new(),
                })
            }
        }
        _ => Err(RpcError {
            code: code::METHOD_NOT_FOUND,
            message: format!("unknown method {method}"),
            id: None,
        }),
    };
    let outcome = match outcome {
        Ok(o) => o,
        Err(mut e) => {
            if e.id.is_none() {
                e.id = id.clone();
            }
            return Err(e);
        }
    };

    // 3. 组响应(仅请求带 id 时返回;通知返回 None)。
    let response = match (&id, outcome.response) {
        (Some(req_id), Some(result)) => {
            Some(json!({ "jsonrpc": "2.0", "id": req_id, "result": result }))
        }
        _ => None,
    };
    Ok(MessageOutcome {
        response,
        events: outcome.events,
    })
}

/// 握手:`bridge/hello` 校验 token + 协议版本,交换能力清单。
fn handle_hello(token: &str, params: &Value) -> Result<MessageOutcome, RpcError> {
    let p = match params.as_object() {
        Some(p) => p,
        None => {
            return Err(RpcError {
                code: code::INVALID_PARAMS,
                message: "params must be an object".into(),
                id: None,
            })
        }
    };

    // token 校验。
    let req_token = p.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if req_token != token {
        return Err(RpcError {
            code: code::UNAUTHORIZED,
            message: "invalid token".into(),
            id: None,
        });
    }

    // 协议版本。
    if let Some(v) = p.get("protocolVersion").and_then(|x| x.as_u64()) {
        if v != PROTOCOL_VERSION as u64 {
            return Err(RpcError {
                code: code::PROTOCOL_MISMATCH,
                message: format!("protocol {v}, expected {PROTOCOL_VERSION}"),
                id: None,
            });
        }
    }

    let dsh_version = p
        .get("dshVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let capabilities: Vec<String> = p
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(MessageOutcome {
        response: Some(json!({
            "zapVersion": env!("CARGO_PKG_VERSION"),
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": ZAP_CAPABILITIES,
        })),
        events: vec![BridgeEvent::Connected {
            dsh_version,
            capabilities,
        }],
    })
}

/// 轻量提取 JSON-RPC 消息头(method / params / id),供 zap.* 异步分流。
fn parse_rpc_head(raw: &str) -> Option<(String, Value, Value)> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let obj = v.as_object()?;
    let method = obj.get("method")?.as_str()?.to_string();
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
    let id = obj.get("id").cloned().unwrap_or(Value::Null);
    Some((method, params, id))
}

/// 分发 zap.* 文件能力方法(同步 IO,根为当前 Zap 项目目录)。
/// 仅在 fast path 内被 spawn_blocking 调用,只接收需要项目根的文件方法;
/// `zap.terminal_context` 不依赖根,经 handle_message 分发。
fn handle_zap_method(method: &str, params: &Value, root: &Path) -> Result<Value, RpcError> {
    match method {
        "zap.list_files" => zap_list_files(params, root),
        "zap.read_file" => zap_read_file(params, root),
        "zap.search" => zap_search(params, root),
        _ => Err(RpcError {
            code: code::METHOD_NOT_FOUND,
            message: format!("unknown method {method}"),
            id: None,
        }),
    }
}

/// 校验相对路径在项目根内,返回规范化后的路径。
///
/// 拒绝绝对路径与 `..` 逃逸;再经 `canonicalize` 防符号链接逃逸到根外。
fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf, RpcError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() || rel.split(['/', '\\']).any(|seg| seg == "..") {
        return Err(RpcError {
            code: code::PATH_INVALID,
            message: format!("path must be relative and inside project root: {rel}"),
            id: None,
        });
    }
    let canon_root = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf());
    let canon = root.join(rel_path).canonicalize().map_err(|err| RpcError {
        code: code::PATH_INVALID,
        message: format!("resolve {rel}: {err}"),
        id: None,
    })?;
    if !canon.starts_with(&canon_root) {
        return Err(RpcError {
            code: code::PATH_INVALID,
            message: format!("path escapes project root: {rel}"),
            id: None,
        });
    }
    Ok(canon)
}

/// 收集从 `root` 到 `dir` 的 gitignore 链(祖先规则传递)+ 全局。
///
/// gitignore 规则祖先生效:根 `.gitignore` 的 `/target` 应影响其所有子目录。
/// 单层实现(`gitignores_for_directory`)不会加载祖先规则,导致进入被忽略
/// 目录时内容仍列出(与 Zap 文件树不一致)。
fn gitignores_up_to_root(dir: &Path, root: &Path) -> Vec<ignore::gitignore::Gitignore> {
    let mut gitignores = Vec::new();
    let mut current = Some(dir);
    while let Some(d) = current {
        let gi = d.join(".gitignore");
        if gi.is_file() {
            gitignores.push(ignore::gitignore::Gitignore::new(gi).0);
        }
        if d == root {
            break;
        }
        current = d.parent();
    }
    let (global, _) = ignore::gitignore::Gitignore::global();
    if !global.is_empty() {
        gitignores.push(global);
    }
    gitignores
}

/// `zap.list_files`:列出相对项目根目录的条目(名字 + 类型)。
fn zap_list_files(params: &Value, root: &Path) -> Result<Value, RpcError> {
    let rel = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
    let dir = resolve_in_root(root, rel)?;
    if !dir.is_dir() {
        return Err(RpcError {
            code: code::NOT_A_DIR,
            message: format!("not a directory: {rel}"),
            id: None,
        });
    }
    let entries = std::fs::read_dir(&dir).map_err(|err| RpcError {
        code: code::READ_DIR,
        message: format!("read dir {rel}: {err}"),
        id: None,
    })?;
    // gitignore 过滤(祖先链 + 全局):gitignore 规则祖先传递(根 .gitignore
    // 的 /target 应影响其子目录),与 Zap 文件树语义一致。
    let gitignores = gitignores_up_to_root(&dir, root);
    let mut list: Vec<Value> = entries
        .flatten()
        .filter(|entry| {
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            !repo_metadata::matches_gitignores(&entry.path(), is_dir, &gitignores, true)
        })
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = match entry.file_type() {
                Ok(t) if t.is_dir() => "dir",
                Ok(t) if t.is_symlink() => "symlink",
                _ => "file",
            };
            json!({ "name": name, "kind": kind })
        })
        .collect();
    list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({ "path": rel, "entries": list }))
}

/// `zap.read_file`:读取相对项目根的文件内容(UTF-8)。
fn zap_read_file(params: &Value, root: &Path) -> Result<Value, RpcError> {
    let rel = params
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| RpcError {
            code: code::INVALID_PARAMS,
            message: "missing path".into(),
            id: None,
        })?;
    let path = resolve_in_root(root, rel)?;
    // 仅普通文件可读:目录/FIFO/设备等 metadata.len() 可能为 0,绕过大小
    // 上限;FIFO 的 read_to_string 还会永久阻塞。先判 is_file 拒绝非普通文件。
    if !path.is_file() {
        return Err(RpcError {
            code: code::READ_FILE,
            message: format!("not a regular file: {rel}"),
            id: None,
        });
    }
    let len = std::fs::metadata(&path).map_err(|err| RpcError {
        code: code::READ_FILE,
        message: format!("stat {rel}: {err}"),
        id: None,
    })?.len();
    if len > MAX_READ_BYTES {
        return Err(RpcError {
            code: code::FILE_TOO_LARGE,
            message: format!("file too large ({len} bytes, max {MAX_READ_BYTES}): {rel}"),
            id: None,
        });
    }
    let content = std::fs::read_to_string(&path).map_err(|err| RpcError {
        code: code::READ_FILE,
        message: format!("read {rel}: {err}"),
        id: None,
    })?;
    Ok(json!({ "path": rel, "content": content }))
}

/// `zap.search`:按文件名子串搜索项目内文件(gitignore 感知,WalkBuilder)。
/// 结果上限 `limit`(默认 100,最多 1000)。
fn zap_search(params: &Value, root: &Path) -> Result<Value, RpcError> {
    let pattern = params
        .get("pattern")
        .and_then(|p| p.as_str())
        .ok_or_else(|| RpcError {
            code: code::INVALID_PARAMS,
            message: "missing pattern".into(),
            id: None,
        })?;
    let limit = params
        .get("limit")
        .and_then(|l| l.as_u64())
        .unwrap_or(100)
        .min(1000) as usize;

    let mut matches: Vec<Value> = Vec::new();
    let walker = WalkBuilder::new(root).hidden(true).build();
    for entry in walker.flatten() {
        if matches.len() >= limit {
            break;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains(pattern) {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();
            matches.push(json!({ "path": rel }));
        }
    }
    Ok(json!({ "pattern": pattern, "matches": matches }))
}

/// `zap.terminal_context`:返回当前活动终端的最近命令(隐私关闭时为空)。
fn zap_terminal_context() -> Result<Value, RpcError> {
    let commands = super::runtime::terminal_context();
    Ok(json!({ "commands": commands }))
}

/// 单条消息处理结果。
#[derive(Debug)]
struct MessageOutcome {
    /// 要返回的 JSON-RPC 响应(通知为 `None`)。
    response: Option<Value>,
    /// 要上抛的事件。
    events: Vec<BridgeEvent>,
}

/// JSON-RPC 错误。
#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
    /// 关联的请求 id(供错误响应);无法解析时 `None`。
    id: Option<Value>,
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
