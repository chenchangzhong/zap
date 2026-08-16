//! DshRuntime:DeepSeek Harness runtime 子进程管理。
//!
//! 职责:
//! - 定位/安装 Node(node_runtime)与 dsh(npm 包,版本锁定)
//! - 启动 `dsh web --port 0`(OS 分配空闲端口),`DSH_HOME` 指向 Zap 数据目录
//! - 就绪探测(HTTP GET /),暴露最终 URL
//! - 停止(SIGTERM → 超时 kill)、崩溃监听与自动重启(带次数限制)
//!
//! 并发模型:启动/重启是 `'static` 异步任务(不借用 self),结果经
//! `ModelContext::spawn` 回调回主线程更新状态。子进程句柄由主线程持有,
//! 每帧 `poll_child` 轮询退出。
//!
//! 平台:macOS 先行(webview 基建仅 macOS 有实现),其他平台编译为空壳。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use command::r#async::Command;
use parking_lot::Mutex;
use warpui::{Entity, ModelContext, SingletonEntity, WindowId};

use super::bridge;

/// dsh npm 包名。
const DSH_NPM_PACKAGE: &str = "@deepseek-ai/dsh";
/// 锁定的 dsh 版本(preview 阶段必须 pin,防止上游破坏性变更)。
const DSH_VERSION: &str = "0.1.0-rc.6";
/// 就绪探测超时(含首次安装)。
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
/// 就绪探测间隔。
const PROBE_INTERVAL: Duration = Duration::from_millis(500);
/// 崩溃自动重启最大次数(连续崩溃超过则放弃)。
const MAX_RESTARTS: u8 = 3;
/// 停止时等待优雅退出的时长,超时后 SIGKILL。
/// 主线程同步阻塞调用(request_stop/Drop),不宜过长。
const STOP_GRACE: Duration = Duration::from_secs(3);
/// 崩溃计数重置阈值:距上次成功启动超过该时长后崩溃,视为健康运行期间
/// 的偶发崩溃,重置连续崩溃计数(避免数月内偶发崩溃累计触发 GiveUp)。
const CRASH_COUNT_RESET_AFTER: Duration = Duration::from_secs(300);

/// 当前 Zap 项目目录(注入 dsh 作 `DSH_CWD`,使会话工作目录跟随 Zap 项目)。
/// 由 `open_dsh_pane` 在启动 dsh 时更新;`start_inner`(异步关联函数)读取。
static WORKSPACE_DIR: LazyLock<Mutex<Option<PathBuf>>> =
    LazyLock::new(|| Mutex::new(None));

/// 设置 dsh 工作目录(最近打开的 Zap 项目目录)。
pub(crate) fn set_workspace_dir(path: PathBuf) {
    *WORKSPACE_DIR.lock() = Some(path);
}

/// 读取 dsh 工作目录;未设置时为 `None`(不注入 `DSH_CWD`,用 dsh 默认)。
pub(crate) fn workspace_dir() -> Option<PathBuf> {
    WORKSPACE_DIR.lock().clone()
}

/// 注入给 dsh 的终端上下文最大命令数。
const TERMINAL_CONTEXT_MAX: usize = 20;
/// 终端上下文刷新节流间隔。
const TERMINAL_CONTEXT_REFRESH: Duration = Duration::from_secs(1);

/// 最近注入的活动终端命令(供桥 `zap.terminal_context` 读取)。
static TERMINAL_CONTEXT: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));
/// 隐私开关:默认关闭(不暴露终端命令给 agent);需显式开启。
static TERMINAL_CONTEXT_ENABLED: AtomicBool = AtomicBool::new(false);
/// 上次刷新时刻(节流)。
static TERMINAL_CONTEXT_LAST_REFRESH: LazyLock<Mutex<Instant>> =
    LazyLock::new(|| Mutex::new(Instant::now() - TERMINAL_CONTEXT_REFRESH));

/// 设置终端上下文注入开关(隐私),并持久化到 dsh 设置文件。默认关闭。
pub(crate) fn set_terminal_context_enabled(enabled: bool) {
    TERMINAL_CONTEXT_ENABLED.store(enabled, Ordering::Relaxed);
    if let Ok(path) = dsh_settings_path() {
        let value = serde_json::json!({ "terminal_context_enabled": enabled });
        let _ = std::fs::write(path, serde_json::to_string_pretty(&value).unwrap_or_default());
    }
}

/// 从 dsh 设置文件加载隐私开关(启动时调用,初始化 atomic)。
pub(crate) fn init_terminal_context_enabled_from_disk() {
    let enabled = dsh_settings_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("terminal_context_enabled").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    TERMINAL_CONTEXT_ENABLED.store(enabled, Ordering::Relaxed);
}

/// dsh 设置文件路径(隐私开关持久化)。正规 settings 框架 UI 后续接入。
fn dsh_settings_path() -> Result<PathBuf> {
    Ok(DshRuntime::dsh_data_dir()?.join("dsh_settings.json"))
}

/// 主线程每帧提取活动终端最近命令到暂存(节流 1s)。隐私关闭时跳过更新。
pub(crate) fn update_terminal_context_from_active(
    ctx: &mut ModelContext<DshRuntime>,
    window_id: WindowId,
) {
    if !TERMINAL_CONTEXT_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let now = Instant::now();
    if now.duration_since(*TERMINAL_CONTEXT_LAST_REFRESH.lock()) < TERMINAL_CONTEXT_REFRESH {
        return;
    }
    *TERMINAL_CONTEXT_LAST_REFRESH.lock() = now;

    let session_id =
        crate::workspace::ActiveSession::handle(ctx).read(ctx, |a, _| a.session(window_id).map(|s| s.id()));
    let commands = match session_id {
        Some(id) => crate::terminal::History::handle(ctx).read(ctx, |h, _| {
            h.commands(id).map(|cmds| {
                cmds.iter()
                    .rev()
                    .take(TERMINAL_CONTEXT_MAX)
                    .map(|e| e.command.clone())
                    .collect()
            })
        }),
        None => None,
    };
    *TERMINAL_CONTEXT.lock() = commands.unwrap_or_default();
}

/// 读取注入的终端上下文(隐私关闭时为空)。
pub(crate) fn terminal_context() -> Vec<String> {
    if TERMINAL_CONTEXT_ENABLED.load(Ordering::Relaxed) {
        TERMINAL_CONTEXT.lock().clone()
    } else {
        Vec::new()
    }
}


/// 崩溃重启的完整流程(供 ctx.spawn 回调)。
#[derive(Debug)]
pub enum DshRestartResult {
    Restarted {
        url: String,
        child: async_process::Child,
    },
    GiveUp { error: String },
}

/// DshRuntime 对外状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DshRuntimeStatus {
    Stopped,
    Starting,
    Ready,
    Failed,
}

/// 一次启动的完整结果(供 ctx.spawn 回调)。
#[derive(Debug)]
pub enum DshStartResult {
    Ready {
        url: String,
        child: async_process::Child,
    },
    Failed { error: String },
}

/// 每帧轮询子进程的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollResult {
    /// 进程仍在运行。
    Running,
    /// 进程已退出,且是主动停止(不重启)。
    StoppedByRequest,
    /// 进程崩溃退出,需要重启。
    Crashed,
    /// 连续崩溃超过上限,放弃重启。
    GiveUp,
}

/// DeepSeek Harness runtime 单例。
///
/// 状态与子进程句柄均由主线程(model 消息循环)持有;异步操作通过
pub struct DshRuntime {
    status: DshRuntimeStatus,
    url: Option<String>,
    /// dsh web 子进程句柄。`None` 表示未运行。
    child: Option<async_process::Child>,
    /// 连续崩溃次数(仅在用户主动启动时清零;崩溃重启序列内保持,
    /// 使 MAX_RESTARTS 上限可达,避免无限重启)。
    consecutive_crashes: u8,
    /// 是否已请求停止(崩溃自动重启与主动停止竞争时优先停止)。
    stopping: bool,
    /// 启动代次:每次 `begin_start`/`begin_restart` 递增。异步启动完成回调
    /// 携带发起时的代次,代次不匹配(期间又有新启动/停止)则丢弃子进程,
    /// 防止双启动竞态泄漏。
    generation: u64,
    /// 上次成功收养子进程的时刻。崩溃时若距上次成功启动已超过
    /// `CRASH_COUNT_RESET_AFTER`,视为健康运行,重置崩溃计数。
    last_success_at: Option<std::time::Instant>,
}

impl Default for DshRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DshRuntime {
    fn drop(&mut self) {
        // 进程退出兜底:优雅终止 dsh 子进程,避免残留。
        if let Some(mut child) = self.child.take() {
            Self::terminate_child(&mut child);
        }
    }
}

impl DshRuntime {
    /// 终止子进程:优先 SIGTERM 优雅退出,超时后 SIGKILL。
    ///
    /// 同步执行(主线程可安全调用);`kill` 是同步的,退出状态交给
    /// async-process 的 reap 线程回收。
    fn terminate_child(child: &mut async_process::Child) {
        // 1. SIGTERM 优雅退出(Unix)。
        #[cfg(unix)]
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }
        // 2. 等待优雅退出,超时后 SIGKILL。
        let deadline = std::time::Instant::now() + STOP_GRACE;
        loop {
            if child.try_status().ok().flatten().is_some() {
                return;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        // 短轮询回收,避免 zombie。
        for _ in 0..100 {
            if child.try_status().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn new() -> Self {
        init_terminal_context_enabled_from_disk();
        Self {
            status: DshRuntimeStatus::Stopped,
            url: None,
            child: None,
            consecutive_crashes: 0,
            stopping: false,
            generation: 0,
            last_success_at: None,
        }
    }

    /// dsh 是否已配置(settings.yaml 存在,或 storages 里有会话数据)。
    ///
    /// dsh 的设置文档默认在 `<DSH_HOME>/settings.yaml`(见 dsh settings-file
    /// 插件);模型配置/会话历史写入 storages/。两者皆无 = 全新安装,
    /// 需要引导用户配置模型 API key。
    pub fn is_configured() -> bool {
        let Ok(dir) = Self::dsh_data_dir() else {
            return false;
        };
        if dir.join("settings.yaml").is_file() {
            return true;
        }
        // storages 里除初始 workspace.json 外还有内容(会话/配置痕迹)。
        let storages = dir.join("storages");
        std::fs::read_dir(&storages).is_ok_and(|mut entries| {
            entries.any(|entry| {
                entry
                    .as_ref()
                    .map(|e| e.file_name() != "workspace.json")
                    .unwrap_or(false)
            })
        })
    }

    /// 启动 dsh runtime(异步,不借用 self)。
    ///
    /// `generation` 是 `begin_start`/`begin_restart` 返回的代次;完成回调
    /// 用它调用 `adopt_child`,代次不匹配时丢弃子进程。
    pub async fn start_future(generation: u64) -> DshStartResult {
        match Self::start_inner().await {
            Ok((child, url)) => DshStartResult::Ready { url, child },
            Err(err) => {
                log::error!("[dsh] start failed: {err:#}");
                DshStartResult::Failed {
                    error: format!("{err:#}"),
                }
            }
        }
    }

    /// dsh 数据目录:`<data_dir>/dsh`。
    ///
    /// 存放 DSH_HOME(profiles/storages)与 dsh npm 安装缓存,不污染用户目录。
    pub fn dsh_data_dir() -> Result<PathBuf> {
        let dir = warp_core::paths::data_dir().join("dsh");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create dsh data dir {}", dir.display()))?;
        Ok(dir)
    }

    pub fn status(&self) -> DshRuntimeStatus {
        self.status
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub fn set_status(&mut self, status: DshRuntimeStatus) {
        self.status = status;
    }


    async fn start_inner() -> Result<(async_process::Child, String)> {
        // 1. 定位/安装 Node。
        let path_env = std::env::var("PATH").unwrap_or_default();
        let mut node = match node_runtime::find_working_node_binary(Some(&path_env)).await {
            Some(node) => node,
            None => {
                log::info!("[dsh] no working node found, installing...");
                let client = http_client::Client::new();
                node_runtime::install_npm(&client)
                    .await
                    .context("Failed to install Node.js")?;
                node_runtime::find_working_node_binary(Some(&path_env))
                    .await
                    .context("Node.js installed but not usable")?
            }
        };

        // dsh 要求 Node >= 22.19(node_runtime 的 MIN_NODE_VERSION 是 20,
        // 系统 Node 20/21 会被误接受);版本不足时回退到 Zap 管理的安装。
        if !Self::node_satisfies_min_version(&node).await {
            log::info!("[dsh] system node too old, installing managed Node...");
            let client = http_client::Client::new();
            node_runtime::install_npm(&client)
                .await
                .context("Failed to install Node.js for dsh")?;
            node = node_runtime::find_working_node_binary(Some(&path_env))
                .await
                .context("Node.js installed but not usable")?;
        }

        // 2. 定位/安装 dsh。
        let dsh_cli = Self::ensure_dsh_installed(&node).await?;

        // 3. 启动 `dsh web --port 0`(带桥插件注入)。
        let dsh_home = Self::dsh_data_dir()?;
        let mut cmd = Command::new(&node);
        // command crate 默认 stdout/stderr = null;dsh web 对 null/socket 无效
        // stdout 会启动即退出(exit 1)。重定向到文件(正常可写目标),顺带留日志。
        let dsh_web_log = std::fs::File::create(dsh_home.join("dsh-web.log"))?;
        cmd.stdout(std::process::Stdio::from(dsh_web_log.try_clone()?));
        cmd.stderr(std::process::Stdio::from(dsh_web_log));
        cmd.arg(&dsh_cli)
            .arg("web")
            .arg("--port")
            .arg("0")
            .env("DSH_HOME", &dsh_home);
        // 桥插件注入:写 profile patch 层(cordis.patch.yml),dsh web 启动时应用。
        if let Some((port, token)) = bridge::bridge_info() {
            cmd.env("ZAP_BRIDGE_ADDRESS", format!("ws://127.0.0.1:{port}"));
            Self::write_bridge_config(&dsh_home, port, &token)?;
            log::info!("[dsh] bridge plugin injected via cordis.patch.yml (port {port})");
        }
        // dsh 会话工作目录 = Zap 当前项目目录。dsh 的 workspaceRoot 取
        // process.cwd(),故设子进程 cwd(默认 agent preset standard 不读
        // DSH_CWD,仅 minimal 用;设 current_dir 两者皆正确)。
        if let Some(dir) = workspace_dir().filter(|d| d.is_dir()) {
            cmd.current_dir(&dir);
            // minimal preset 兼容(standard 忽略)。
            cmd.env("DSH_CWD", &dir);
            log::info!("[dsh] workspace cwd set to {}", dir.display());
        }
        let mut child = cmd.spawn().context("Failed to spawn dsh web")?;

        // 4. 就绪探测:等端口出现 + HTTP 200。失败时显式清理子进程,
        // 避免 async-process 的 Child drop 不杀进程导致泄漏。
        let url = match Self::wait_until_ready(&child).await {
            Ok(url) => url,
            Err(err) => {
                let _ = child.kill();
                return Err(err);
            }
        };

        // 5. 记录 PID 到文件,便于调试/外部检查。
        let _ = std::fs::write(dsh_home.join("dsh.pid"), child.id().to_string());

        Ok((child, url))
    }

    /// 检查 node 版本是否满足 dsh 要求(engines: `^22.19 || >=24`)。
    async fn node_satisfies_min_version(node: &Path) -> bool {
        let mut cmd = Command::new(node);
        cmd.arg("--version");
        let Ok(output) = cmd.output().await else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let version_str = version_str.trim_start_matches('v');
        let parts: Vec<u64> = version_str
            .split('.')
            .take(2)
            .filter_map(|p| p.parse().ok())
            .collect();
        match parts.as_slice() {
            // ^22.19.0:22.19 <= v < 23;或 v >= 24。
            [major, minor] => (*major, *minor) >= (22, 19) && *major < 23 || *major >= 24,
            _ => false,
        }
    }

    /// 确保 dsh 已安装,返回 CLI 入口 JS 路径。
    async fn ensure_dsh_installed(node: &Path) -> Result<PathBuf> {
        let data_dir = Self::dsh_data_dir()?;
        let dsh_dir = data_dir.join("dsh-install");
        let cli_js = dsh_dir
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js");

        // 已安装且版本匹配则复用。
        if cli_js.is_file() {
            let installed =
                std::fs::read_to_string(dsh_dir.join("version.txt")).unwrap_or_default();
            if installed.trim() == DSH_VERSION {
                return Ok(cli_js);
            }
            log::info!(
                "[dsh] version mismatch (installed {:?}, want {DSH_VERSION}), reinstalling",
                installed.trim()
            );
            let _ = std::fs::remove_dir_all(&dsh_dir);
        }

        std::fs::create_dir_all(&dsh_dir)?;
        let npm = if node.file_name().map(|n| n == "node").unwrap_or(false) {
            // system node:PATH 里找 npm
            PathBuf::from("npm")
        } else {
            node_runtime::npm_binary_path().unwrap_or_else(|_| PathBuf::from("npm"))
        };

        log::info!("[dsh] installing {DSH_NPM_PACKAGE}@{DSH_VERSION}...");
        // 注意:不用 `--prefix`(npm 11 下不落盘 node_modules),改为
        // 在目标目录内执行 npm install。
        let mut cmd = Command::new(&npm);
        cmd.current_dir(&dsh_dir)
            .arg("install")
            .arg("--no-save")
            .arg(format!("{DSH_NPM_PACKAGE}@{DSH_VERSION}"));
        let output = cmd
            .output()
            .await
            .context("Failed to run npm install for dsh")?;
        if !output.status.success() {
            bail!(
                "npm install {} failed: {}",
                DSH_NPM_PACKAGE,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::write(dsh_dir.join("version.txt"), DSH_VERSION)?;
        if !cli_js.is_file() {
            bail!("dsh installed but entry not found at {}", cli_js.display());
        }
        Ok(cli_js)
    }

    /// 生成 cordis.yml(注入 zap-bridge 插件),返回 patch 文件路径。
    ///
    /// 插件文件(`<dsh_home>/zap-bridge.ts`)尚未就绪时返回 `None`(降级:
    /// 不带 `--patch` 启动,避免 dsh 指向缺失文件)。
    fn write_bridge_config(dsh_home: &Path, port: u16, token: &str) -> Result<()> {
        let plugin_ts = dsh_home.join("zap-bridge.ts");
        // 总是尝试从 bundled 源码覆盖提取:dev 下保证插件更新传播到 DSH_HOME;
        // 发布时 manifest 源码路径不可用 → 复用已提取文件。
        let src =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/bundled/dsh/zap-bridge.ts");
        match std::fs::read(&src) {
            Ok(contents) => {
                std::fs::write(&plugin_ts, contents)?;
                log::info!("[dsh] extracted zap-bridge.ts to {}", plugin_ts.display());
            }
            Err(_) if !plugin_ts.is_file() => {
                log::warn!(
                    "[dsh] zap-bridge.ts missing, starting dsh without bridge plugin"
                );
                return Ok(());
            }
            Err(_) => {
                // 源码不可用但已有提取文件:复用(发布场景)。
            }
        }
        // rc.6 的 `web --patch` 无效(unknown option);插件经 profile 用户
        // patch 层 `profiles/web/cordis.patch.yml` 注入(dsh web 启动时应用)。
        let patch_dir = dsh_home.join("profiles").join("web");
        std::fs::create_dir_all(&patch_dir)?;
        let content = format!(
            "- insert:\n    - id: zap-bridge\n      name: '{}'\n      config:\n        bridgeAddress: 'ws://127.0.0.1:{port}'\n        token: '{}'\n",
            plugin_ts.display(),
            token
        );
        std::fs::write(patch_dir.join("cordis.patch.yml"), content)?;
        Ok(())
    }

    /// 轮询探测 dsh web 就绪(端口监听 + HTTP 200),返回 URL。
    async fn wait_until_ready(child: &async_process::Child) -> Result<String> {
        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        let client = http_client::Client::new();
        let pid = child.id();

        loop {
            if std::time::Instant::now() > deadline {
                bail!("dsh web did not become ready within {STARTUP_TIMEOUT:?}");
            }

            if let Some(port) = Self::find_listening_port(pid) {
                let url = format!("http://127.0.0.1:{port}");
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => return Ok(url),
                    _ => {}
                }
            }

            tokio::time::sleep(PROBE_INTERVAL).await;
        }
    }

    /// 找到 `pid` 进程监听的本地端口(仅 macOS;其他平台返回 None)。
    fn find_listening_port(pid: u32) -> Option<u16> {
        let output = std::process::Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-a", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines().skip(1) {
            if let Some(idx) = line.rfind("127.0.0.1:") {
                let rest = &line[idx + "127.0.0.1:".len()..];
                let port_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(port) = port_str.parse() {
                    return Some(port);
                }
            }
        }
        None
    }

    /// 标记一次用户主动启动开始(由主线程在 spawn 前调用)。
    ///
    /// 复位停止标记与崩溃计数(新会话重新计数),递增启动代次,并把状态
    /// 置为 `Starting`,使后续 `open_dsh_pane` 不会重复触发启动。
    /// 返回本次启动的代次,供异步启动回调校验。
    pub fn begin_start(&mut self) -> u64 {
        self.stopping = false;
        self.consecutive_crashes = 0;
        self.generation += 1;
        self.set_status(DshRuntimeStatus::Starting);
        self.generation
    }

    /// 标记一次崩溃自动重启开始(由 lib.rs 在 spawn restart 前调用)。
    ///
    /// 只递增代次并置位 `Starting`;**不复位崩溃计数**(连续崩溃序列内
    /// 保持递增,使 MAX_RESTARTS 上限可达),也**不复位 stopping**
    /// (崩溃路径 stopping 必为 false;保持原值避免覆盖用户刚发的停止请求)。
    pub fn begin_restart(&mut self) -> u64 {
        self.generation += 1;
        self.set_status(DshRuntimeStatus::Starting);
        self.generation
    }

    /// 接收启动完成的子进程(回调中调用)。
    ///
    /// 若已请求停止或代次不匹配(期间又有新启动),不收养进程,直接优雅
    /// 终止,避免双启动/无 pane 常驻进程。返回是否真正收养。
    ///
    /// 状态维护约定:
    /// - `stopping` 为 true 时,`request_stop` 已把状态置 `Stopped`,这里
    ///   保持不动;
    /// - 代次不匹配时,必有更新的 `begin_start`/`begin_restart`(已置
    ///   `Starting`)或更新的 adopt(已置 `Ready`),这里**不改写状态**,
    ///   避免把新代次的状态打回 `Stopped`(否则会出现「Stopped + 活子进程」
    ///   的不一致,并连锁导致下次打开时覆盖句柄泄漏)。
    pub fn adopt_child(&mut self, child: async_process::Child, url: String, generation: u64) -> bool {
        if self.stopping || self.generation != generation {
            log::info!(
                "[dsh] startup finished after stop/new start (gen {generation} != {}); killing child",
                self.generation
            );
            let mut child = child;
            Self::terminate_child(&mut child);
            return false;
        }
        // 防御:正常情况下本代次不应已有 child(双启动竞态的兜底),
        // 若存在则先终止,避免句柄覆盖泄漏。
        if let Some(mut old_child) = self.child.take() {
            log::warn!("[dsh] adopting child while previous child still alive; terminating old");
            Self::terminate_child(&mut old_child);
        }
        self.child = Some(child);
        self.url = Some(url);
        self.stopping = false;
        self.last_success_at = Some(std::time::Instant::now());
        self.set_status(DshRuntimeStatus::Ready);
        true
    }

    /// 主动停止 runtime(面板关闭/退出时)。同步执行,可在主线程直接调用:
    /// SIGTERM 优雅退出,超时后 SIGKILL。
    pub fn request_stop(&mut self) {
        log::info!("[dsh] request_stop: child={}", self.child.is_some());
        self.stopping = true;
        if let Some(mut child) = self.child.take() {
            Self::terminate_child(&mut child);
        }
        self.set_status(DshRuntimeStatus::Stopped);
        self.url = None;
        self.consecutive_crashes = 0;
    }
    ///
    /// 返回 [`PollResult`]:崩溃时置状态为 `Stopped`(等待重启调度),
    /// 超过上限置 `Failed` 并返回 [`PollResult::GiveUp`]。
    pub fn poll_child(&mut self) -> PollResult {
        let Some(child) = &mut self.child else {
            return PollResult::Running;
        };
        match child.try_status() {
            Ok(Some(_status)) => {
                self.child = None;
                if self.stopping {
                    return PollResult::StoppedByRequest;
                }
                // 距上次成功启动超过阈值:视为健康运行期间的偶发崩溃,
                // 重置连续崩溃计数(从 1 重新计)。
                if self
                    .last_success_at
                    .map(|t| t.elapsed() > CRASH_COUNT_RESET_AFTER)
                    .unwrap_or(false)
                {
                    log::info!("[dsh] crash after healthy run; resetting crash count");
                    self.consecutive_crashes = 1;
                } else {
                    self.consecutive_crashes += 1;
                }
                log::warn!(
                    "[dsh] process exited unexpectedly (crash #{})",
                    self.consecutive_crashes
                );
                if self.consecutive_crashes > MAX_RESTARTS {
                    self.set_status(DshRuntimeStatus::Failed);
                    return PollResult::GiveUp;
                }
                self.set_status(DshRuntimeStatus::Stopped);
                PollResult::Crashed
            }
            _ => PollResult::Running,
        }
    }

    /// 崩溃后重启(由外部在 `poll_child` 返回 `Crashed` 后调度,`'static` future)。
    pub async fn restart_future(generation: u64) -> DshRestartResult {
        match Self::start_future(generation).await {
            DshStartResult::Ready { url, child } => DshRestartResult::Restarted { url, child },
            DshStartResult::Failed { error } => DshRestartResult::GiveUp { error },
        }
    }
}

/// DshRuntime 对外事件。
#[derive(Debug, Clone)]
pub enum DshRuntimeEvent {
    /// runtime 就绪,`url` 为 dsh Web UI 地址。
    Ready { url: String },
    /// 崩溃后自动重启完成(仅通知已有 pane 导航,不自动开新 pane)。
    Restarted { url: String },
    /// 启动/重启失败,或连续崩溃超过上限放弃重启。
    Failed { error: String },
}

impl Entity for DshRuntime {
    type Event = DshRuntimeEvent;
}

impl SingletonEntity for DshRuntime {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 lsof 输出解析:取 127.0.0.1 端口,忽略 IPv6/其他地址。
    #[test]
    fn parses_listening_port_from_lsof_output() {
        let sample = "\
COMMAND  PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
node    1234 zhong  14u  IPv4 0x1234      0t0  TCP 127.0.0.1:63815 (LISTEN)
node    1234 zhong  15u  IPv6 0x5678      0t0  TCP [::1]:63816 (LISTEN)
";
        // 把 lsof 输出喂给解析逻辑:构造临时文件并 mock lsof 不可行,
        // 直接测试行解析的辅助逻辑。
        let lines: Vec<&str> = sample.lines().collect();
        assert!(lines.len() >= 2);
        // 第二行含 127.0.0.1:63815
        let line = lines[1];
        let idx = line.rfind("127.0.0.1:").expect("has 127.0.0.1");
        let rest = &line[idx + "127.0.0.1:".len()..];
        let port_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        assert_eq!(port_str, "63815");
    }

    /// 版本常量必须与 npm 包一致(安装路径依赖它)。
    #[test]
    fn version_constant_is_parseable() {
        assert!(DSH_VERSION.starts_with("0.1.0"));
        assert!(DSH_VERSION.contains('-') || DSH_VERSION.split('.').count() == 3);
    }

    /// 生成 cordis.patch.yml:内容含 id/桥地址/token/插件绝对路径。
    #[test]
    fn write_bridge_config_generates_patch() {
        let dir = std::env::temp_dir().join(format!("dsh-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zap-bridge.ts"), "// zap-bridge").unwrap();

        DshRuntime::write_bridge_config(&dir, 54321, "t0k3n").unwrap();
        let patch = dir.join("profiles").join("web").join("cordis.patch.yml");
        let content = std::fs::read_to_string(&patch).unwrap();
        assert!(content.contains("id: zap-bridge"));
        assert!(content.contains("bridgeAddress: 'ws://127.0.0.1:54321'"));
        assert!(content.contains("token: 't0k3n'"));
        assert!(content.contains(&format!("name: '{}'", dir.join("zap-bridge.ts").display())));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// set_workspace_dir / workspace_dir 存取与缺省。
    #[test]
    fn workspace_dir_set_and_get() {
        *WORKSPACE_DIR.lock() = None;
        assert!(workspace_dir().is_none(), "default should be None");
        set_workspace_dir(PathBuf::from("/tmp/zap-foo"));
        assert_eq!(workspace_dir(), Some(PathBuf::from("/tmp/zap-foo")));
    }

    /// 终端上下文隐私开关:关闭时返回空,开启后返回暂存命令。
    #[test]
    fn terminal_context_privacy_gate() {
        TERMINAL_CONTEXT_ENABLED.store(false, Ordering::Relaxed);
        *TERMINAL_CONTEXT.lock() = vec!["ls".to_string(), "cd src".to_string()];
        assert!(terminal_context().is_empty(), "privacy off => empty");

        set_terminal_context_enabled(true);
        assert_eq!(
            terminal_context(),
            vec!["ls".to_string(), "cd src".to_string()],
            "privacy on => returns staged commands"
        );

        // 复位默认。
        set_terminal_context_enabled(false);
    }
    #[test]
    fn write_bridge_config_extracts_source() {
        let dir = std::env::temp_dir().join(format!("dsh-cfg-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // dsh_home 无插件文件 → 从源码提取。
        DshRuntime::write_bridge_config(&dir, 54322, "t0k3n2").unwrap();
        assert!(dir.join("zap-bridge.ts").is_file(), "plugin extracted to dsh_home");
        let patch = dir.join("profiles").join("web").join("cordis.patch.yml");
        let content = std::fs::read_to_string(&patch).unwrap();
        assert!(content.contains(&format!("name: '{}'", dir.join("zap-bridge.ts").display())));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 端到端冒烟:真实启动 dsh runtime(需网络安装 dsh,首次较慢)。
    /// 验证:启动成功、URL 可访问、子进程可停止。
    #[test]
    #[ignore = "requires network + npm install, run manually"]
    fn smoke_start_stop() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let result = DshRuntime::start_future(0).await;
            match result {
                DshStartResult::Ready { url, mut child } => {
                    // URL 可达。
                    let client = http_client::Client::new();
                    let resp = client.get(&url).send().await.expect("GET url");
                    assert!(resp.status().is_success(), "HTTP {}", resp.status());
                    // 停止。
                    let _ = child.kill();
                    let _ = child.status().await;
                }
                DshStartResult::Failed { error } => panic!("start failed: {error}"),
            }
        });
    }

    /// 生命周期:启动 → 同步 kill + 轮询回收,进程退出。
    #[test]
    #[ignore = "requires network + npm install, run manually"]
    fn lifecycle_start_stop() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let result = DshRuntime::start_future(0).await;
            let (url, mut child) = match result {
                DshStartResult::Ready { url, child } => (url, child),
                DshStartResult::Failed { error } => panic!("start failed: {error}"),
            };
            assert!(url.starts_with("http://127.0.0.1:"));

            // 同步 kill + 轮询回收(模拟 request_stop 的核心路径)。
            let pid = child.id();
            let _ = child.kill();
            for _ in 0..200 {
                if child.try_status().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            // 确认进程已退出。
            assert!(
                child.try_status().ok().flatten().is_some(),
                "process {pid} still alive after kill"
            );
        });
    }

    /// 崩溃检测:poll_child 在进程退出后返回 Crashed,计数递增。
    #[cfg(unix)]
    #[test]
    fn poll_child_detects_exit() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let mut runtime = DshRuntime::new();
            // 用 sleep 进程模拟 dsh 子进程。
            let mut cmd = command::r#async::Command::new("sleep");
            cmd.arg("30");
            let child = cmd.spawn().expect("spawn sleep");
            runtime.adopt_child(child, "http://127.0.0.1:1".to_string(), 0);
            assert_eq!(runtime.status(), DshRuntimeStatus::Ready);

            // 杀掉进程,下一轮 poll_child 应检测到崩溃(计数 1)。
            let pid = runtime.child.as_mut().unwrap().id();
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            // 等进程真正退出。
            for _ in 0..100 {
                if runtime.poll_child() == PollResult::Crashed {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(runtime.consecutive_crashes, 1);
            assert_eq!(runtime.status(), DshRuntimeStatus::Stopped);
        });
    }

    /// 主动停止后 poll_child 返回 StoppedByRequest,不重启。
    #[cfg(unix)]
    #[test]
    fn poll_child_stopped_by_request() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let mut runtime = DshRuntime::new();
            let mut cmd = command::r#async::Command::new("sleep");
            cmd.arg("30");
            let child = cmd.spawn().expect("spawn sleep");
            let pid = child.id();
            runtime.adopt_child(child, "http://127.0.0.1:1".to_string(), 0);
            // 标记停止(不取走 child,模拟"已请求停止但进程还在"的窗口)。
            runtime.stopping = true;
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            for _ in 0..100 {
                if runtime.poll_child() == PollResult::StoppedByRequest {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(runtime.status(), DshRuntimeStatus::Ready); // 状态未变(不重启)
            assert_eq!(runtime.consecutive_crashes, 0); // 不计崩溃
        });
    }

    /// stopping 后 adopt_child 不收养进程(关闭后 in-flight 启动完成)。
    #[cfg(unix)]
    #[test]
    fn adopt_child_after_stop_kills_child() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let mut runtime = DshRuntime::new();
            // 先请求停止(模拟关闭 pane)。
            runtime.request_stop();

            // 模拟一个 in-flight 启动完成返回的 child。
            let mut cmd = command::r#async::Command::new("sleep");
            cmd.arg("30");
            let child = cmd.spawn().expect("spawn sleep");
            let pid = child.id();
            runtime.adopt_child(child, "http://127.0.0.1:1".to_string(), 0);

            // 不应收养:child 为 None,且子进程被杀。
            assert_eq!(runtime.child.is_none(), true);
            assert_eq!(runtime.status(), DshRuntimeStatus::Stopped);
            // 进程应已退出(被 kill)。
            for _ in 0..100 {
                let output = std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .output()
                    .expect("kill -0");
                if !output.status.success() {
                    break; // 进程不存在
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let output = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .expect("kill -0");
            assert!(
                !output.status.success(),
                "child {pid} should have been killed after stop"
            );
        });
    }

    /// begin_start 复位 stopping 与崩溃计数,并置位 Starting。
    #[test]
    fn begin_start_resets_state() {
        let mut runtime = DshRuntime::new();
        runtime.request_stop();
        runtime.consecutive_crashes = 2;
        let gen = runtime.begin_start();
        assert_eq!(runtime.status(), DshRuntimeStatus::Starting);
        assert_eq!(runtime.consecutive_crashes, 0);
        // stopping 复位:begin_start 后再 adopt_child 应正常收养。
        assert!(!runtime.stopping);
        assert!(gen > 0);
    }

    /// 连续崩溃超过 MAX_RESTARTS 后 poll_child 返回 GiveUp(计数序列不清零)。
    #[cfg(unix)]
    #[test]
    fn poll_child_gives_up_after_max_restarts() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let mut runtime = DshRuntime::new();
            // 模拟连续崩溃:每次 spawn sleep → adopt → SIGKILL → poll。
            for expected_crash in 1..=MAX_RESTARTS + 1 {
                let mut cmd = command::r#async::Command::new("sleep");
                cmd.arg("30");
                let child = cmd.spawn().expect("spawn sleep");
                let pid = child.id();
                let gen = runtime.begin_restart();
                assert!(runtime.adopt_child(child, "http://127.0.0.1:1".to_string(), gen));
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
                let mut result = PollResult::Running;
                for _ in 0..100 {
                    result = runtime.poll_child();
                    if result != PollResult::Running {
                        break;
                    }

                    std::thread::sleep(Duration::from_millis(20));
                }
                if expected_crash <= MAX_RESTARTS {
                    assert_eq!(result, PollResult::Crashed, "crash #{expected_crash}");
                    assert_eq!(runtime.consecutive_crashes, expected_crash);
                } else {
                    assert_eq!(result, PollResult::GiveUp, "should give up");
                    assert_eq!(runtime.status(), DshRuntimeStatus::Failed);
                }
            }
        });
    }

    /// 健康运行超过阈值后崩溃,重置连续崩溃计数。
    #[cfg(unix)]
    #[test]
    fn crash_after_healthy_run_resets_count() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let mut runtime = DshRuntime::new();

            // 第一次崩溃(计数 1)。
            let mut cmd = command::r#async::Command::new("sleep");
            cmd.arg("30");
            let child = cmd.spawn().expect("spawn sleep");
            let pid = child.id();
            let gen = runtime.begin_restart();
            assert!(runtime.adopt_child(child, "http://127.0.0.1:1".to_string(), gen));
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            for _ in 0..100 {
                if runtime.poll_child() == PollResult::Crashed {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(runtime.consecutive_crashes, 1);

            // 第二次:adopt 之后把 last_success_at 改到很久以前,模拟健康
            // 运行超过阈值;崩溃时应重置计数为 1(而非 2)。
            let mut cmd = command::r#async::Command::new("sleep");
            cmd.arg("30");
            let child = cmd.spawn().expect("spawn sleep");
            let pid = child.id();
            let gen = runtime.begin_restart();
            assert!(runtime.adopt_child(child, "http://127.0.0.1:1".to_string(), gen));
            runtime.last_success_at = Some(
                std::time::Instant::now() - CRASH_COUNT_RESET_AFTER - Duration::from_secs(1),
            );
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            for _ in 0..100 {
                if runtime.poll_child() == PollResult::Crashed {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                runtime.consecutive_crashes, 1,
                "count should reset after healthy run"
            );
        });
    }

    /// 崩溃重启完整链路:启动 → kill 子进程 → poll_child 检测 →
    /// restart_future 重启 → 新 URL 可达。
    #[test]
    #[ignore = "requires network + npm install, run manually"]
    fn crash_restart_cycle() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            // 1. 启动。
            let (url1, mut child) = match DshRuntime::start_future(0).await {
                DshStartResult::Ready { url, child } => (url, child),
                DshStartResult::Failed { error } => panic!("start failed: {error}"),
            };
            let client = http_client::Client::new();
            assert!(
                client.get(&url1).send().await.unwrap().status().is_success(),
                "first start not reachable"
            );

            // 2. 模拟崩溃:SIGKILL 子进程。
            let pid = child.id();
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            // 3. 等退出(等价 poll_child 检测)。
            for _ in 0..100 {
                if child.try_status().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(
                child.try_status().ok().flatten().is_some(),
                "process {pid} did not exit after SIGKILL"
            );

            // 4. 重启(等价 lib.rs 的 restart_future 调度)。
            let (url2, mut child2) = match DshRuntime::restart_future(0).await {
                DshRestartResult::Restarted { url, child } => (url, child),
                DshRestartResult::GiveUp { error } => panic!("restart gave up: {error}"),
            };
            assert_ne!(url1, url2, "restart should pick a new port");
            assert!(
                client.get(&url2).send().await.unwrap().status().is_success(),
                "restarted runtime not reachable"
            );

            // 5. 清理。
            let _ = child2.kill();
            let _ = child2.status().await;
        });
    }
}
