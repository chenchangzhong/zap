//! DshRuntime:DeepSeek Harness runtime 子进程管理。
//!
//! 职责:
//! - 定位 PATH 中的全局 `dsh` 命令(由用户自行安装/升级,Zap 不做安装/版本管理)
//! - 启动 Zap 专属 profile(`dsh --profile zap --port 0`,OS 分配空闲端口),
//!   `DSH_HOME` 指向 `~/.dsh`
//! - 就绪探测(解析 dsh 输出的 URL 并 HTTP 探测),暴露最终 URL
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

/// Zap 专属 dsh profile 名(`$DSH_HOME/profiles/<name>`;由 desktop profile
/// 复制而来,与用户终端自用的 web profile、DSH Desktop 的 desktop profile
/// 互不干扰,插件/配置各自独立)。
const DSH_PROFILE: &str = "zap";
/// dsh npm 包名(升级命令与 registry 查询共用)。
pub(crate) const DSH_NPM_PACKAGE: &str = "@deepseek-ai/dsh";
/// 就绪探测超时。
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

/// 从 dsh 设置文件加载隐私开关(启动时调用,初始化 atomic)。
pub(crate) fn init_terminal_context_enabled_from_disk() {
    TERMINAL_CONTEXT_ENABLED.store(terminal_context_enabled_from_disk(), Ordering::Relaxed);
}

/// 从 dsh 设置文件实时读取隐私开关值。
fn terminal_context_enabled_from_disk() -> bool {
    dsh_settings_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("terminal_context_enabled").and_then(|b| b.as_bool()))
        .unwrap_or(false)
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
    // 从文件实时刷新隐私开关:用户运行中改 dsh_settings.json 后 1s 内生效
    // (仅启动时缓存会导致改文件不生效)。
    TERMINAL_CONTEXT_ENABLED.store(terminal_context_enabled_from_disk(), Ordering::Relaxed);
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
            let cmds = h.commands(id);
            log::debug!(
                "[dsh] terminal ctx: session {id:?}, history commands = {:?}",
                cmds.as_ref().map(|c| c.len())
            );
            cmds.map(|cmds| {
                cmds.iter()
                    .rev()
                    .take(TERMINAL_CONTEXT_MAX)
                    .map(|e| e.command.clone())
                    .collect()
            })
        }),
        None => {
            log::debug!("[dsh] terminal ctx: no active session for window {window_id:?}");
            None
        }
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

/// 全局 dsh 更新检查结果(仅提示用,不参与启动流程)。
#[derive(Debug, Clone)]
pub enum DshUpdateCheck {
    /// 无更新,或检查失败/离线(静默,不打扰用户)。
    UpToDate,
    /// registry 有比当前已装版本更新的 semver 版本。
    UpdateAvailable {
        installed: String,
        latest: String,
    },
}

/// registry 版本是否比已装版本新。两边都需为合法 semver(预发布号按
/// semver 规则排序,如 0.1.2-rc.1 < 0.1.2);任一不可解析则视为无法
/// 比较,静默不提示。
fn is_update_available(latest: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(installed),
    ) {
        (Ok(latest), Ok(installed)) => latest > installed,
        _ => false,
    }
}

/// 本会话是否已做过 dsh 更新检查(每次打开 pane 触发,一次即可)。
static UPDATE_CHECKED: AtomicBool = AtomicBool::new(false);

/// 是否应执行本会话的 dsh 更新检查(首次调用 true 并置位,之后 false)。
pub fn should_check_update_now() -> bool {
    !UPDATE_CHECKED.swap(true, Ordering::Relaxed)
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
    /// 运行 Zap 专属 profile(`dsh --profile zap --port 0 --no-open`)。
    /// dsh 由用户自行安装/升级,Zap 不做安装与版本管理(启动前无需网络检查)。
    ///
    /// 代次校验不在本函数内:调用方持 `begin_start`/`begin_restart` 返回的
    /// 代次,完成回调里传给 `adopt_child` 做代次比对,代次不匹配时丢弃子进程。
    pub async fn start_future() -> DshStartResult {
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

    /// 非阻塞检查全局 dsh 是否有新版本(仅提示,不参与启动流程)。
    ///
    /// 比较 `dsh --version` 与 npm registry latest(并行执行,registry 查询
    /// 5s 超时)。dsh 不存在、命令失败、离线或版本不可解析时一律返回
    /// `UpToDate`(静默;dsh 不存在时启动路径已有明确报错)。
    pub async fn check_update_future() -> DshUpdateCheck {
        let Ok(dsh_bin) = Self::find_global_dsh() else {
            return DshUpdateCheck::UpToDate;
        };
        let (latest, installed) = tokio::join!(
            Self::query_latest_version(),
            Self::installed_version(&dsh_bin)
        );
        let (Some(latest), Ok(installed)) = (latest, installed) else {
            return DshUpdateCheck::UpToDate;
        };
        if is_update_available(&latest, &installed) {
            log::info!("[dsh] update available: {installed} -> {latest}");
            DshUpdateCheck::UpdateAvailable { installed, latest }
        } else {
            DshUpdateCheck::UpToDate
        }
    }

    /// 读取全局 dsh 版本号(`dsh --version`)。
    async fn installed_version(dsh_bin: &Path) -> Result<String> {
        let mut cmd = Command::new(dsh_bin);
        cmd.arg("--version");
        let output = cmd.output().await.context("Failed to run dsh --version")?;
        if !output.status.success() {
            bail!("dsh --version exited with {}", output.status);
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// 查询 npm registry 上 dsh 的最新版本号。
    ///
    /// 失败(网络不可达 / 响应异常 / 超时)返回 `None`,调用方静默跳过。
    /// reqwest 默认无总超时,故用 `tokio::time::timeout` 兜底。
    async fn query_latest_version() -> Option<String> {
        const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
        let client = http_client::Client::new();
        let url = format!("https://registry.npmjs.org/{DSH_NPM_PACKAGE}/latest");
        let result = tokio::time::timeout(CHECK_TIMEOUT, async {
            let resp = client.get(&url).send().await.ok()?;
            if !resp.status().is_success() {
                log::warn!("[dsh] update check failed: HTTP {}", resp.status());
                return None;
            }
            let body: serde_json::Value = resp.json().await.ok()?;
            body.get("version").and_then(|v| v.as_str()).map(str::to_string)
        })
        .await;
        match result {
            Ok(version) => version,
            Err(_) => {
                log::warn!("[dsh] update check timed out after {CHECK_TIMEOUT:?}");
                None
            }
        }
    }

    /// dsh 数据目录:`~/.dsh`(与用户终端 dsh、DSH Desktop 共用的 DSH_HOME;
    /// Zap 通过专属 profile 隔离,见 [`DSH_PROFILE`])。
    ///
    /// 存放 DSH_HOME(profiles/storages)与运行日志,不污染用户目录。
    /// 首次使用时把旧位置 `~/.zap/dsh` 的数据一次性迁移过来。
    pub fn dsh_data_dir() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .with_context(|| "Failed to resolve home dir for dsh data dir")?;
        let dir = home.join(".dsh");
        Self::migrate_legacy_data_dir(&dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create dsh data dir {}", dir.display()))?;
        Ok(dir)
    }

    /// 一次性迁移旧数据目录 `~/.zap/dsh` → `~/.dsh`。
    ///
    /// 仅当新目录不存在且旧目录存在时执行;`rename` 失败(dsh 正在运行持有
    /// 句柄等)不阻塞,打日志后继续用新目录重建。
    fn migrate_legacy_data_dir(new_dir: &Path) {
        let Some(home) = dirs::home_dir() else { return };
        let legacy = home.join(".zap").join("dsh");
        if !legacy.is_dir() || new_dir.exists() {
            return;
        }
        if let Some(parent) = new_dir.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        match std::fs::rename(&legacy, new_dir) {
            Ok(()) => {
                log::info!(
                    "[dsh] migrated data dir {} -> {}",
                    legacy.display(),
                    new_dir.display()
                );
            }
            Err(err) => {
                log::warn!(
                    "[dsh] migrate data dir {} -> {} failed: {err}",
                    legacy.display(),
                    new_dir.display()
                );
            }
        }
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
        // 1. 定位全局 dsh 命令(用户自装自升级,Zap 不做安装/版本管理)。
        let dsh_bin = Self::find_global_dsh()?;
        log::info!("[dsh] using global dsh at {}", dsh_bin.display());

        // 2. 启动 Zap 专属 profile(`dsh --profile zap --port 0`,带客户端
        //    插件注入;`--profile web` 是 `dsh web` 的等价形式,Zap 用自己的
        //    profile 隔离插件与配置)。
        let dsh_home = Self::dsh_data_dir()?;
        // 注入 zap-bridge-client 浏览器端插件(经 webview IPC 发项目切换通知)。
        if let Err(err) = Self::install_client_plugin(&dsh_home) {
            log::warn!("[dsh] install_client_plugin failed: {err}");
        }
        let mut cmd = Command::new(&dsh_bin);
        // command crate 默认 stdout/stderr = null;dsh web 对 null/socket 无效
        // stdout 会启动即退出(exit 1)。重定向到文件(正常可写目标),顺带留日志。
        let dsh_web_log = std::fs::File::create(dsh_home.join("dsh-web.log"))?;
        cmd.stdout(std::process::Stdio::from(dsh_web_log.try_clone()?));
        cmd.stderr(std::process::Stdio::from(dsh_web_log));
        cmd.arg("--profile")
            .arg(DSH_PROFILE)
            .arg("--port")
            .arg("0")
            // 不自动打开系统浏览器(Zap 用自己的 webview 承载 dsh UI)。
            .arg("--no-open")
            .env("DSH_HOME", &dsh_home);
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

        // 4. 就绪探测:dsh 启动成功后会把最终 URL(含随机端口与访问 token)
        //    打到 stdout(已重定向到 dsh-web.log),解析出该 URL 并 HTTP 探测。
        //    失败时显式清理子进程,避免 async-process 的 Child drop 不杀进程
        //    导致泄漏。
        let log_path = dsh_home.join("dsh-web.log");
        let url = match Self::wait_until_ready(&log_path).await {
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

    /// 在 PATH 中定位全局 `dsh` 命令(Zap 不管理安装/版本,由用户自装自升级)。
    ///
    /// 返回绝对路径而非依赖 execvp 隐式 PATH 解析:便于日志记录实际命中的
    /// dsh 位置,且找不到时能给出行 actionable 的错误信息。
    fn find_global_dsh() -> Result<PathBuf> {
        let path_env = std::env::var("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path_env) {
            let candidate = dir.join("dsh");
            if Self::is_executable_file(&candidate) {
                return Ok(candidate);
            }
        }
        bail!(
            "dsh command not found in PATH; install it globally first, \
             e.g. `npm install -g @deepseek-ai/dsh`"
        )
    }

    /// 路径存在且为可执行文件。
    fn is_executable_file(path: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            return std::fs::metadata(path)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
        }
        #[cfg(not(unix))]
        {
            std::fs::metadata(path).is_ok_and(|m| m.is_file())
        }
    }

    /// 注入 zap-bridge-client 浏览器端插件到 dsh 并注册 cordis patch。
    /// 客户端插件经 webview IPC 发项目切换通知给 Zap。
    ///
    /// Zap 的 profile 由 desktop profile 复制而来,patch 文件已带用户插件的
    /// 配置,故第 2 步只幂等追加 zap-bridge-client 条目,不覆写整个文件。
    fn install_client_plugin(dsh_home: &Path) -> Result<()> {
        // 1. 写入客户端插件文件到 DSH_HOME/node_modules。
        let node_modules = dsh_home.join("node_modules").join("@zap").join("zap-bridge-client");
        std::fs::create_dir_all(&node_modules)?;
        const CLIENT_INDEX: &str = include_str!("../../assets/bundled/dsh/zap-bridge-client-index.js");
        const CLIENT_JS: &str = include_str!("../../assets/bundled/dsh/zap-bridge-client.js");
        const CLIENT_PKG: &str = include_str!("../../assets/bundled/dsh/zap-bridge-client-package.json");
        std::fs::write(node_modules.join("index.js"), CLIENT_INDEX)?;
        std::fs::write(node_modules.join("client.js"), CLIENT_JS)?;
        std::fs::write(node_modules.join("package.json"), CLIENT_PKG)?;
        log::info!("[dsh] installed zap-bridge-client to {}", node_modules.display());
        // 2. 向 profile 的 cordis.patch.yml 幂等追加 zap-bridge-client 条目
        //    (dsh 启动时加载)。
        let patch_dir = dsh_home.join("profiles").join(DSH_PROFILE);
        std::fs::create_dir_all(&patch_dir)?;
        // patch 行 name 必须是包名:client-modules 用
        // `require.resolve('<name>/package.json')` 解析 client 声明,
        // 文件路径无法解析会导致 entry 静默不入表、客户端插件永不加载。
        const INSERT_ENTRY: &str =
            "- insert:\n    - id: zap-bridge-client\n      name: '@zap/zap-bridge-client'\n";
        let patch_path = patch_dir.join("cordis.patch.yml");
        let existing = std::fs::read_to_string(&patch_path).unwrap_or_default();
        if existing.contains("zap-bridge-client") {
            return Ok(());
        }
        let mut content = existing;
        // dsh 生成的空 patch 是 flow 风格 `[]`,直接追加块条目会成非法 YAML。
        if content.trim() == "[]" {
            content = String::new();
        } else if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(INSERT_ENTRY);
        std::fs::write(&patch_path, content)?;
        Ok(())
    }

    /// 轮询等待 dsh web 就绪并返回其 URL。
    ///
    /// dsh 启动成功后把最终 URL(随机端口 + 访问 token)打到 stdout(已重
    /// 定向到 dsh-web.log):`dsh web: http://127.0.0.1:<port>/?token=<token>`。
    /// 0.1.2-rc.1 起 web 端有 token 鉴权,裸地址 GET 返回 401,故必须解析
    /// 该 URL 用作探测与 webview 加载地址;URL 中的 token 随启动随机生成。
    async fn wait_until_ready(log_path: &Path) -> Result<String> {
        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        // 探活专用客户端,与业务 http_client 隔离:
        // - 不跟重定向:token URL 响应 303 → Location `/` 并种认证 cookie,
        //   跟随后落地 401(浏览器场景由 cookie 会话兜住,探活只看服务是否
        //   应答),若跟随会永远探测失败;
        // - 不走代理(loopback 不应经代理);
        // - 单次请求限时,任何一环挂死只损失一个探测周期,不会卡死启动。
        let probe = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .context("Failed to build dsh probe client")?;

        let mut last_probe_log = String::new();
        loop {
            if std::time::Instant::now() > deadline {
                bail!("dsh web did not become ready within {STARTUP_TIMEOUT:?}");
            }

            if let Some(url) = Self::read_ready_url(log_path) {
                match probe.get(&url).send().await {
                    // 2xx 直接就绪;303(种 cookie 跳 `/`)同样视为服务已就绪。
                    Ok(resp)
                        if resp.status().is_success() || resp.status().is_redirection() =>
                    {
                        return Ok(url)
                    }
                    Ok(resp) => {
                        let state = format!("status {}", resp.status());
                        if last_probe_log != state {
                            log::info!("[dsh] probe: url={url} unexpected {state}");
                            last_probe_log = state;
                        }
                    }
                    Err(err) => {
                        let state = format!("error {err}");
                        if last_probe_log != state {
                            log::info!("[dsh] probe: url={url} {state}");
                            last_probe_log = state;
                        }
                    }
                }
            }

            tokio::time::sleep(PROBE_INTERVAL).await;
        }
    }

    /// 从 dsh-web.log 读取就绪 URL(尚无输出时返回 None)。
    fn read_ready_url(log_path: &Path) -> Option<String> {
        Self::parse_ready_url(&std::fs::read_to_string(log_path).ok()?)
    }

    /// 从日志文本解析 dsh 启动器打印的 web UI 就绪 URL。
    ///
    /// 锚定启动器自身的 `dsh web: ` 输出行:profile 插件也可能往 stdout 打
    /// 自己的 `http://127.0.0.1:` URL 且先于 web UI 输出,取"第一个
    /// 127.0.0.1 URL"会解析到无关地址导致探测永远失败。
    fn parse_ready_url(text: &str) -> Option<String> {
        const MARKER: &str = "dsh web: http://127.0.0.1:";
        let idx = text.find(MARKER)?;
        let rest = &text[idx + "dsh web: ".len()..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(rest[..end].to_string())
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
    pub async fn restart_future() -> DshRestartResult {
        match Self::start_future().await {
            DshStartResult::Ready { url, child } => DshRestartResult::Restarted { url, child },
            DshStartResult::Failed { error } => DshRestartResult::GiveUp { error },
        }
    }
}

impl Entity for DshRuntime {
    type Event = super::bridge::BridgeEvent;
}

impl SingletonEntity for DshRuntime {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证就绪 URL 解析:锚定启动器的 `dsh web: ` 行,忽略插件先输出的
    /// 其他 127.0.0.1 URL。
    #[test]
    fn parses_ready_url_from_log_text() {
        let sample = "noise before\n\
                      [some-plugin] page http://127.0.0.1:3080/qr\n\
                      dsh web: http://127.0.0.1:64536/?token=hkVHyTzitwrF-wWC7NpO00F7u\n\
                      trailing line\n";
        assert_eq!(
            DshRuntime::parse_ready_url(sample).as_deref(),
            Some("http://127.0.0.1:64536/?token=hkVHyTzitwrF-wWC7NpO00F7u")
        );
        // 无 `dsh web: ` 行时不误取插件 URL。
        assert_eq!(
            DshRuntime::parse_ready_url("some-plugin page http://127.0.0.1:3080/qr"),
            None
        );
        assert_eq!(DshRuntime::parse_ready_url("no url here"), None);
    }

    /// 更新判定:semver 比较(预发布号 < 正式版),不可解析时静默不提示。
    #[test]
    fn update_available_comparison() {
        assert!(is_update_available("0.1.3", "0.1.2-rc.1"));
        assert!(is_update_available("0.1.2", "0.1.2-rc.1"));
        assert!(!is_update_available("0.1.2-rc.1", "0.1.2-rc.1"));
        assert!(!is_update_available("0.1.2-rc.1", "0.1.2"));
        assert!(!is_update_available("0.1.1", "0.1.2-rc.1"));
        assert!(!is_update_available("not-semver", "0.1.2"));
        assert!(!is_update_available("0.1.2", "not-semver"));
    }

    /// 已移除安装/版本管理:dsh 由用户全局安装,Zap 仅定位 PATH 中的命令。

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

        TERMINAL_CONTEXT_ENABLED.store(true, Ordering::Relaxed);
        assert_eq!(
            terminal_context(),
            vec!["ls".to_string(), "cd src".to_string()],
            "privacy on => returns staged commands"
        );

        // 复位默认。
        TERMINAL_CONTEXT_ENABLED.store(false, Ordering::Relaxed);
    }
    /// 端到端冒烟:真实启动 dsh runtime(需网络安装 dsh,首次较慢)。
    /// 验证:启动成功、URL 可访问、子进程可停止。
    #[test]
    #[ignore = "requires global dsh in PATH, run manually"]
    fn smoke_start_stop() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let result = DshRuntime::start_future().await;
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
    #[ignore = "requires global dsh in PATH, run manually"]
    fn lifecycle_start_stop() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let result = DshRuntime::start_future().await;
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
    #[ignore = "requires global dsh in PATH, run manually"]
    fn crash_restart_cycle() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            // 1. 启动。
            let (url1, mut child) = match DshRuntime::start_future().await {
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
            let (url2, mut child2) = match DshRuntime::restart_future().await {
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
