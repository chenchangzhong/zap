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
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use command::r#async::Command;
use parking_lot::Mutex;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, WindowId};

use crate::{
    features::FeatureFlag, pane_group::PaneGroup, terminal::omp_models::get_user_env,
    undo_close::UndoCloseStack,
};

/// Zap 专属 dsh profile 名(`$DSH_HOME/profiles/<name>`;由 desktop profile
/// 复制而来,与用户终端自用的 web profile、DSH Desktop 的 desktop profile
/// 互不干扰,插件/配置各自独立)。
const DSH_PROFILE: &str = "zap";
/// Zap 专属 dsh web 日志文件名前缀(带 zap 前缀,与用户终端 dsh、DSH Desktop
/// 的日志区分开;位于 DSH_HOME 根目录)。
///
/// 每次启动写一份 `zap-dsh-web-<unix 秒>[-<序号>].log`,只保留最近
/// [`DSH_WEB_LOG_KEEP`] 份:单文件覆盖写会把上一次启动的现场(尤其崩溃那次)
/// 直接截掉,事后无法回看。
const DSH_WEB_LOG_PREFIX: &str = "zap-dsh-web-";
/// 保留的历史 web 日志份数(含当前一份)。
const DSH_WEB_LOG_KEEP: usize = 5;
/// 记录本次启动 `(zap_pid, pgid, started_at)` 的文件名,供下次启动回收残留实例。
const DSH_PID_FILE: &str = "dsh.pid";
/// dsh npm 包名(升级命令与 registry 查询共用)。
pub(crate) const DSH_NPM_PACKAGE: &str = "@deepseek-ai/dsh";
/// 参与更新检查的发布渠道(dist-tag);`next` 为 rc 预览、`alpha` 为内测,
/// 均不占用 `latest`。toast 按此顺序逐渠道提示,哪个渠道升级由用户自选;
/// 渠道开关(About 页)可关掉部分渠道。
pub(crate) const UPDATE_CHANNELS: [&str; 3] = ["latest", "next", "alpha"];
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

/// 当前 dsh 进程组 id(= spawn 时 dsh 的 pid;dsh 以 `setpgid(0,0)` 自成一组的
/// 组长)。信号 handler 在异步信号上下文里读它,故只能原子读写、不能取锁;
/// `0` 表示当前没有受管的 dsh。
static DSH_PROCESS_GROUP: AtomicI32 = AtomicI32::new(0);

/// 记录当前受管的 dsh 进程组(信号 handler 转发用)。
fn remember_process_group(pgid: u32) {
    DSH_PROCESS_GROUP.store(pgid as i32, Ordering::SeqCst);
}

/// 清除当前受管的 dsh 进程组记录。
///
/// 只在记录的正是 `pgid` 时清除:新旧启动重叠时,回收旧组不能把新组的记录抹掉。
fn forget_process_group(pgid: u32) {
    let _ = DSH_PROCESS_GROUP.compare_exchange(
        pgid as i32,
        0,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

/// 当前 unix 时间戳(秒);系统时钟早于 epoch 时返回 0。
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// 文件名是否为本模块写出的 web 日志。
fn is_web_log_name(name: &str) -> bool {
    name.starts_with(DSH_WEB_LOG_PREFIX) && name.ends_with(".log")
}

/// 安装「把终止信号转发给 dsh 进程组」的 handler(进程内只装一次)。
///
/// Zap 被 `pkill`(SIGTERM)、被 SIGKILL 或崩溃时既不跑 `Drop` 也不跑
/// `on_will_terminate`,信号是唯一的同步清理机会;没有它,dsh 会被 reparent
/// 到 launchd 后继续常驻(开发重启循环下每次重启留一个)。
///
/// handler 内只做异步信号安全的操作:原子读 + `killpg` + 恢复默认动作并重抛,
/// 不取锁、不分配。末尾的重抛是必须的——装了 handler 后默认动作被替换,不重抛
/// `pkill` 就杀不掉 Zap(开发重启循环会失效)。
#[cfg(unix)]
fn install_signal_forwarding() {
    use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
    use std::sync::Once;

    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        for signal in [SIGTERM, SIGINT, SIGHUP] {
            let handler = move || {
                let pgid = DSH_PROCESS_GROUP.load(Ordering::SeqCst);
                if pgid > 0 {
                    // 用 kill(-pgid) 而不是 killpg:POSIX 的异步信号安全函数表里
                    // 明确列了 kill,而 killpg 只是它的薄封装(两者等价)。
                    // SAFETY:kill 是异步信号安全的,参数只来自原子读。
                    unsafe {
                        libc::kill(-pgid, libc::SIGTERM);
                    }
                }
                let _ = signal_hook::low_level::emulate_default_handler(signal);
            };
            // SAFETY:handler 内只调用异步信号安全的函数(killpg / raise /
            // sigaction),且只读原子量,不触碰任何加锁状态。
            let registered = unsafe { signal_hook::low_level::register(signal, handler) };
            if let Err(err) = registered {
                log::warn!("[dsh] failed to forward signal {signal} to dsh: {err}");
            }
        }
    });
}

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

/// 各更新渠道的启用开关(持久化在 `dsh_settings.json` 的
/// `dsh_update_channels` 对象;缺省全开)。关闭后 toast 检查与 About 页
/// 渠道检查都不再包含该渠道。
pub(crate) fn dsh_channel_enabled(channel: &str) -> bool {
    dsh_settings_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| {
            v.get("dsh_update_channels")
                .and_then(|c| c.get(channel))
                .and_then(|b| b.as_bool())
        })
        .unwrap_or(true)
}

/// 写入单个更新渠道开关(读改写 settings JSON,保留其它键)。
pub(crate) fn set_dsh_channel_enabled(channel: &str, enabled: bool) -> Result<()> {
    let path = dsh_settings_path()?;
    let mut root: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    root["dsh_update_channels"][channel] = serde_json::Value::Bool(enabled);
    std::fs::write(&path, serde_json::to_string_pretty(&root)?)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
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

/// 一次 dsh 渠道检查的完整快照(About 页"检查更新"用)。
#[derive(Debug, Clone, Default)]
pub struct DshChannelsSnapshot {
    /// 已装版本(`dsh --version`);dsh 不在 PATH 或命令失败时为 `None`。
    pub installed: Option<String>,
    /// 启用渠道的 (渠道, registry 最新版本),渠道顺序同 [`UPDATE_CHANNELS`];
    /// registry 查询失败/超时/全渠道关闭时为空。
    pub channels: Vec<(&'static str, String)>,
}

/// 全局 dsh 更新检查结果(仅提示用,不参与启动流程)。
#[derive(Debug, Clone)]
pub enum DshUpdateCheck {
    /// 无更新,或检查失败/离线(静默,不打扰用户;下次打开 pane 会重查)。
    UpToDate,
    /// registry 至少一个渠道(dist-tag)有比当前已装版本更新的 semver 版本。
    UpdateAvailable {
        installed: String,
        /// 有更新的 (渠道, 版本) 列表,渠道顺序同 [`UPDATE_CHANNELS`]。
        available: Vec<(&'static str, String)>,
    },
}

/// registry 版本是否比已装版本新。两边都需为合法 semver(预发布号按
/// semver 规则排序,如 0.1.2-rc.1 < 0.1.2);任一不可解析则视为无法
/// 比较,静默不提示。
pub(crate) fn is_update_available(latest: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(installed),
    ) {
        (Ok(latest), Ok(installed)) => latest > installed,
        _ => false,
    }
}

/// 指定渠道(latest/next/alpha)的全局 dsh 升级命令:`latest` 走默认 tag,
/// 预览渠道显式带 `@tag`。About 页升级链接与 toast 点击共用。
pub(crate) fn upgrade_command(channel: &str) -> String {
    if channel == "latest" {
        format!("npm install -g {DSH_NPM_PACKAGE}")
    } else {
        format!("npm install -g {DSH_NPM_PACKAGE}@{channel}")
    }
}

/// 从 dist-tags 查询结果中选出已装版本落后的渠道及其版本(渠道顺序同
/// [`UPDATE_CHANNELS`];该渠道缺失或版本比对不通过时跳过)。
fn channel_updates(installed: &str, tags: &[(&'static str, String)]) -> Vec<(&'static str, String)> {
    tags.iter()
        .filter(|(channel, latest)| {
            UPDATE_CHANNELS.contains(channel) && is_update_available(latest, installed)
        })
        .map(|(channel, latest)| (*channel, latest.clone()))
        .collect()
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
    /// 最近一次启动/重启失败的原因(供 pane 展示与复制);成功启动或
    /// 主动停止时清空,避免展示过期错误。
    error: Option<String>,
    /// 是否已登记「待停止」(`DshPane::detach(Closed)` 或 undo 条目过期时置位,
    /// 见 `mark_stop_pending` / `stop_if_no_dsh_pane`)。
    ///
    /// 不在此刻直接终止子进程:dsh 是全局单例,而 pane 被关闭后仍可能在 undo
    /// 宽限期内被撤销恢复(关 tab 或关窗口两条路径),关窗时机也可能早于/晚于
    /// 新实例的启动——`open_dsh_pane` 会复用现有实例,直接杀会误伤。真正的终止
    /// 由 `poll_pending_stop` 在确认「已无任何**可达**的 dsh pane」
    /// (`has_reachable_dsh_pane`)后执行。
    stop_pending: bool,
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
    /// 终止 dsh 子进程**组**:优先 SIGTERM 优雅退出,超时后 SIGKILL。
    ///
    /// dsh 以 `setpgid(0,0)` 自成一组的组长(spawn 时 pgid == pid),故这里按
    /// 整组发信号:dsh 自己派生的子进程(agent shell、MCP server 等)一并回收,
    /// 不会在 dsh 退出后变成孤儿。
    ///
    /// 同步执行(主线程可安全调用);`kill` 是同步的,退出状态交给
    /// async-process 的 reap 线程回收。
    fn terminate_child(child: &mut async_process::Child) {
        let pgid = child.id();
        // 1. SIGTERM 优雅退出(整组)。
        Self::signal_process_group(pgid, true);
        // 2. 等待优雅退出,超时后 SIGKILL。
        let deadline = std::time::Instant::now() + STOP_GRACE;
        while child.try_status().ok().flatten().is_none() {
            if std::time::Instant::now() >= deadline {
                Self::signal_process_group(pgid, false);
                // 兜底单进程 SIGKILL:非 unix 平台没有进程组。
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // 3. 短轮询回收,避免 zombie。
        for _ in 0..100 {
            if child.try_status().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        forget_process_group(pgid);
    }

    /// 向 dsh 进程组发终止信号:`graceful` 为 true 时 SIGTERM,否则 SIGKILL。
    ///
    /// 非 unix 平台没有进程组(见 `command::r#async::Command::new_with_process_group`
    /// 的 TODO),此时为空操作,由 `child.kill()` 兜底。
    fn signal_process_group(pgid: u32, graceful: bool) {
        #[cfg(unix)]
        unsafe {
            libc::killpg(
                pgid as i32,
                if graceful {
                    libc::SIGTERM
                } else {
                    libc::SIGKILL
                },
            );
        }
        #[cfg(not(unix))]
        {
            let _ = (pgid, graceful);
        }
    }

    /// 把本次启动的 `(zap_pid, pgid, started_at)` 写入 `dsh.pid`。
    ///
    /// 覆盖写单份:下次启动靠它判断「上一次 Zap 是否已死、它的 dsh 组是否还在」。
    /// 记录里必须带 zap_pid——只存 dsh pid 时,无法区分「上次 Zap 遗留的实例」
    /// 与「另一个还活着的 Zap 实例(打包版 + dev 版并存)的 dsh」。
    fn write_pid_record(dir: &Path, pgid: u32) {
        let text = format!(
            "zap_pid={}\npgid={pgid}\nstarted_at={}\n",
            std::process::id(),
            unix_now()
        );
        let path = dir.join(DSH_PID_FILE);
        if let Err(err) = std::fs::write(&path, text) {
            log::warn!("[dsh] failed to write {}: {err}", path.display());
        }
    }

    /// 启动前回收上一次 Zap 会话遗留的 dsh 进程组。
    ///
    /// Zap 被 `pkill`(SIGTERM)、被 SIGKILL 或崩溃退出时,`Drop` 与
    /// `on_will_terminate` 都不执行,上次启动的 dsh 无人回收、会一直常驻
    /// (开发重启循环下每次重启留一个)。这里按 [`DSH_PID_FILE`] 记录判定并整组
    /// 回收,判定条件见 [`Self::orphan_group_from_record`]。
    ///
    /// 记录丢失(旧格式、文件被删)时不做进程扫描兜底:扫描只能靠
    /// `--profile zap` 匹配 argv,会连用户手工启动的同 profile 实例一起杀掉,
    /// 代价高于收益。
    async fn reclaim_orphan_group(dir: &Path) {
        let Ok(text) = std::fs::read_to_string(dir.join(DSH_PID_FILE)) else {
            return;
        };
        let Some(pgid) = Self::orphan_group_from_record(&text) else {
            return;
        };
        if !Self::process_group_alive(pgid) {
            // 上次 Zap 已正常停止(组不存在):无需动作,也保持日志干净。
            return;
        }
        log::warn!(
            "[dsh] reclaiming orphan dsh process group {pgid}: previous Zap run exited without cleanup"
        );
        Self::signal_process_group(pgid as u32, true);
        let deadline = std::time::Instant::now() + STOP_GRACE;
        while Self::process_group_alive(pgid) {
            if std::time::Instant::now() >= deadline {
                Self::signal_process_group(pgid as u32, false);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 从上次启动的记录里解出「该回收的 dsh 进程组」。
    ///
    /// 三个条件同时成立才回收:记录可解析、记录的 Zap 进程**已不存在**、
    /// 且不是本进程自己的记录。第二个条件保证多实例并存时不会误杀另一个活着的
    /// Zap 的 dsh;旧格式(裸 pid、无 pgid)解析不出,一律不回收——宁可不回收,
    /// 也不赌 pid 复用后误杀无关进程组。
    fn orphan_group_from_record(text: &str) -> Option<i32> {
        let zap_pid = Self::pid_record_field(text, "zap_pid")?;
        let pgid = Self::pid_record_field(text, "pgid")?;
        if pgid <= 0 || zap_pid == std::process::id() as i32 {
            return None;
        }
        if Self::process_alive(zap_pid) {
            return None;
        }
        Some(pgid)
    }

    /// 取记录文本里某个 `key=value` 整数域。
    fn pid_record_field(text: &str, key: &str) -> Option<i32> {
        text.lines()
            .filter_map(|line| line.trim().split_once('='))
            .find(|(name, _)| *name == key)
            .and_then(|(_, value)| value.trim().parse().ok())
    }

    /// 进程是否仍存在(`kill(pid, 0)` 探活,`EPERM` 视为存在)。
    fn process_alive(pid: i32) -> bool {
        #[cfg(unix)]
        {
            if unsafe { libc::kill(pid, 0) } == 0 {
                return true;
            }
            std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            false
        }
    }

    /// 进程组是否仍存在(`killpg(pgid, 0)` 探活;非 unix 恒 false)。
    fn process_group_alive(pgid: i32) -> bool {
        #[cfg(unix)]
        {
            unsafe { libc::killpg(pgid, 0) == 0 }
        }
        #[cfg(not(unix))]
        {
            let _ = pgid;
            false
        }
    }

    /// 只保留最近 [`DSH_WEB_LOG_KEEP`] 份 web 日志(按修改时间),删除更旧的。
    ///
    /// `current` 是本次刚创建的那份,**永不删除**:同秒内重启产生的带序号文件
    /// (`<秒>-2.log`)按路径比较排在 `<秒>.log` 之前,只按时间+路径排序时会把
    /// 最新那份当成最旧的删掉——而崩溃重启恰恰常发生在同一秒内。
    fn prune_web_logs(dir: &Path, current: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut logs: Vec<(SystemTime, PathBuf)> = entries
            .flatten()
            .filter(|entry| is_web_log_name(&entry.file_name().to_string_lossy()))
            .filter(|entry| entry.path() != current)
            .filter_map(|entry| {
                let modified = entry.metadata().and_then(|meta| meta.modified()).ok()?;
                Some((modified, entry.path()))
            })
            .collect();
        // current 自己占一份配额,其余最多再留 KEEP - 1 份。
        let allowed = DSH_WEB_LOG_KEEP.saturating_sub(1);
        if logs.len() <= allowed {
            return;
        }
        // 主序修改时间,同刻(同一秒内多次启动)退化为按路径(即文件名)比较,
        // 保证结果确定、不随 read_dir 顺序漂移。
        logs.sort_by(|(a_time, a_path), (b_time, b_path)| {
            a_time.cmp(b_time).then_with(|| a_path.cmp(b_path))
        });
        let stale = logs.len() - allowed;
        for (_, path) in logs.into_iter().take(stale) {
            let _ = std::fs::remove_file(&path);
        }
    }

    /// 最近一份 web 日志路径(崩溃原因提取用;目录里没有任何日志时返回 None)。
    fn latest_web_log_path(dir: &Path) -> Option<PathBuf> {
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|entry| is_web_log_name(&entry.file_name().to_string_lossy()))
            .filter_map(|entry| {
                let modified = entry.metadata().and_then(|meta| meta.modified()).ok()?;
                Some((modified, entry.path()))
            })
            .max_by(|(a_time, a_path), (b_time, b_path)| {
                a_time.cmp(b_time).then_with(|| a_path.cmp(b_path))
            })
            .map(|(_, path)| path)
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
            error: None,
            stop_pending: false,
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

    /// 非阻塞检查全局 dsh 各发布渠道是否有新版本(仅提示,不参与启动流程)。
    ///
    /// 比较 `dsh --version` 与 npm registry 上已启用渠道(latest/next/alpha,
    /// 渠道开关见 [`dsh_channel_enabled`])的版本(并行执行,registry 查询
    /// 5s 超时)。任一启用渠道有更新即列入结果,每个渠道各弹一条 toast,
    /// 升不升级、走哪个渠道由用户自选。dsh 不存在、命令失败、离线或版本
    /// 不可解析时一律返回 `UpToDate`(静默;dsh 不存在时启动路径已有明确报错)。
    pub async fn check_update_future() -> DshUpdateCheck {
        let snapshot = Self::check_channels_future().await;
        let Some(installed) = snapshot.installed else {
            return DshUpdateCheck::UpToDate;
        };
        let available = channel_updates(&installed, &snapshot.channels);
        if available.is_empty() {
            DshUpdateCheck::UpToDate
        } else {
            log::info!("[dsh] update available: {installed} -> {available:?}");
            DshUpdateCheck::UpdateAvailable { installed, available }
        }
    }

    /// 一次完整的 dsh 渠道检查快照(About 页"检查更新"展示用):已装版本
    /// + 各启用渠道的 registry 最新版本。与 toast 检查共用同一次查询。
    pub async fn check_channels_future() -> DshChannelsSnapshot {
        let dsh_bin = Self::find_global_dsh().ok();
        let (tags, installed) = tokio::join!(
            Self::query_dist_tags(),
            async move {
                match dsh_bin {
                    Some(bin) => Self::installed_version(&bin).await.ok(),
                    None => None,
                }
            }
        );
        DshChannelsSnapshot {
            installed,
            channels: tags.into_iter().filter(|(c, _)| dsh_channel_enabled(c)).collect(),
        }
    }

    /// 读取全局 dsh 版本号(`dsh --version`)。
    async fn installed_version(dsh_bin: &Path) -> Result<String> {
        let mut cmd = Command::new(dsh_bin);
        cmd.arg("--version");
        // 与 start_inner 同因:`dsh` 的 shebang 需要 PATH 中的 node,GUI
        // 环境缺失时更新检查会静默失效。
        Self::apply_shebang_path_env(&mut cmd);
        let output = cmd.output().await.context("Failed to run dsh --version")?;
        if !output.status.success() {
            bail!("dsh --version exited with {}", output.status);
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// 查询 npm registry 上 dsh 各发布渠道(latest/next/alpha)的版本号。
    ///
    /// 走官方 dist-tags 端点(`/-/package/<pkg>/dist-tags`),一次拿全部
    /// 渠道且响应极小。失败(网络不可达 / 响应异常 / 超时)返回空列表,
    /// 调用方静默跳过。reqwest 默认无总超时,故用 `tokio::time::timeout` 兜底。
    async fn query_dist_tags() -> Vec<(&'static str, String)> {
        const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
        let client = http_client::Client::new();
        let url = format!("https://registry.npmjs.org/-/package/{DSH_NPM_PACKAGE}/dist-tags");
        let result = tokio::time::timeout(CHECK_TIMEOUT, async {
            let resp = client.get(&url).send().await.ok()?;
            if !resp.status().is_success() {
                log::warn!("[dsh] update check failed: HTTP {}", resp.status());
                return None;
            }
            let body: serde_json::Value = resp.json().await.ok()?;
            let tags = body.as_object()?;
            Some(
                UPDATE_CHANNELS
                    .iter()
                    .filter_map(|channel| {
                        tags.get(*channel)
                            .and_then(|v| v.as_str())
                            .map(|v| (*channel, v.to_string()))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .await;
        match result {
            Ok(tags) => tags.unwrap_or_default(),
            Err(_) => {
                log::warn!("[dsh] update check timed out after {CHECK_TIMEOUT:?}");
                Vec::new()
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

    /// 最近一次启动/重启失败的原因(无失败时为 `None`)。
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_status(&mut self, status: DshRuntimeStatus) {
        self.status = status;
    }

    /// 记录一次启动/重启失败(原因 + `Failed` 状态)。
    ///
    /// 状态与原因一并写入,避免调用方先 `set_status` 再改 error 时中间态被
    /// 订阅者读到(emit 后的订阅回调只依赖本方法的结果)。
    pub fn set_failed(&mut self, error: String) {
        self.error = Some(error);
        self.set_status(DshRuntimeStatus::Failed);
    }

    async fn start_inner() -> Result<(async_process::Child, String)> {
        // 1. 定位全局 dsh 命令(用户自装自升级,Zap 不做安装/版本管理)。
        let dsh_bin = Self::find_global_dsh()?;
        log::info!("[dsh] using global dsh at {}", dsh_bin.display());

        // 2. 启动 Zap 专属 profile(`dsh --profile zap --port 0`,带客户端
        //    插件注入;`--profile web` 是 `dsh web` 的等价形式,Zap 用自己的
        //    profile 隔离插件与配置)。
        let dsh_home = Self::dsh_data_dir()?;
        // 2.1 先回收上一次 Zap 会话遗留的 dsh 组:Zap 非正常退出时没有任何清理
        //     路径会执行,不回收就会一代代堆积(见 reclaim_orphan_group)。
        Self::reclaim_orphan_group(&dsh_home).await;
        // 注入 zap-bridge-client 浏览器端插件(经 webview IPC 发项目切换通知)。
        if let Err(err) = Self::install_client_plugin(&dsh_home) {
            log::warn!("[dsh] install_client_plugin failed: {err}");
        }
        // 进程组隔离:让 dsh 成为自成一组的组长(pgid == pid),停止与启动回收
        // 都按整组处理,dsh 派生的子进程不会变成孤儿。
        let mut cmd = Command::new_with_process_group(&dsh_bin);
        // command crate 默认 stdout/stderr = null;dsh web 对 null/socket 无效
        // stdout 会启动即退出(exit 1)。重定向到文件(正常可写目标),顺带留日志。
        // 每次启动一份独立文件:同秒内再次启动(崩溃重启)加序号,不截断上一份。
        let mut log_path = dsh_home.join(format!("{DSH_WEB_LOG_PREFIX}{}.log", unix_now()));
        let mut seq = 2;
        while log_path.exists() {
            log_path = dsh_home.join(format!("{DSH_WEB_LOG_PREFIX}{}-{seq}.log", unix_now()));
            seq += 1;
        }
        let dsh_web_log = std::fs::File::create(&log_path)?;
        Self::prune_web_logs(&dsh_home, &log_path);
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
        Self::apply_shebang_path_env(&mut cmd);
        // 装一次信号转发:Zap 被 pkill/崩溃杀掉时没有其它清理机会(见
        // install_signal_forwarding)。只在真要拉起 dsh 时装,不影响不开 dsh 的会话。
        #[cfg(unix)]
        install_signal_forwarding();
        let mut child = cmd.spawn().context("Failed to spawn dsh web")?;
        // 刚 spawn 就登记进程组并落盘:启动期崩溃或 Zap 自己被杀时也有据可查
        // (DSH_PROCESS_GROUP 供信号 handler 转发)。
        remember_process_group(child.id());
        Self::write_pid_record(&dsh_home, child.id());

        // 4. 就绪探测:dsh 启动成功后会把最终 URL(含随机端口与访问 token)
        //    打到 stdout(已重定向到本次启动的日志文件),解析出该 URL 并 HTTP 探测。
        //    探测同时盯着子进程,启动期退出立即失败。
        //    失败时显式清理进程组,避免 async-process 的 Child drop 不杀进程
        //    导致泄漏。
        let url = match Self::wait_until_ready(&mut child, &log_path).await {
            Ok(url) => url,
            Err(err) => {
                // 启动期失败:整组强杀(dsh 可能已派生过子进程,只杀组长会留孤儿)。
                // 不等优雅退出——这里在异步任务里,不能阻塞执行器;万一仍有残留,
                // 已落盘的 pid 记录会让下次启动回收它。
                Self::signal_process_group(child.id(), false);
                let _ = child.kill();
                forget_process_group(child.id());
                return Err(err);
            }
        };

        Ok((child, url))
    }

    /// 在 PATH 中定位全局 `dsh` 命令(Zap 不管理安装/版本,由用户自装自升级)。
    ///
    /// 返回绝对路径而非依赖 execvp 隐式 PATH 解析:便于日志记录实际命中的
    /// dsh 位置,且找不到时能给出行 actionable 的错误信息。
    fn find_global_dsh() -> Result<PathBuf> {
        let path_env = std::env::var("PATH").unwrap_or_default();
        if let Some(path) = Self::find_dsh_in_path(&path_env) {
            return Ok(path);
        }
        // Finder/Dock 启动的 GUI app 不继承 shell 配置的 PATH(nvm、homebrew
        // 等),回退到登录 shell 抓取的环境变量里再查一次(对齐 omp 的做法)。
        for (key, val) in get_user_env() {
            if key == "PATH" {
                if let Some(path) = Self::find_dsh_in_path(&val) {
                    return Ok(path);
                }
            }
        }
        bail!(
            "dsh command not found in PATH; install it globally first, \
             e.g. `npm install -g @deepseek-ai/dsh`"
        )
    }

    /// 在冒号分隔的 PATH 字符串中查找可执行的 `dsh`,返回首个命中目录下的路径。
    fn find_dsh_in_path(path_env: &str) -> Option<PathBuf> {
        Self::find_executable_in_path(path_env, "dsh")
    }

    /// 在冒号分隔的 PATH 字符串中查找可执行文件,返回首个命中目录下的路径。
    fn find_executable_in_path(path_env: &str, name: &str) -> Option<PathBuf> {
        std::env::split_paths(path_env)
            .map(|dir| dir.join(name))
            .find(|candidate| Self::is_executable_file(candidate))
    }

    /// 为 dsh 子进程注入能解析其 shebang(`#!/usr/bin/env node`)的 PATH。
    ///
    /// Finder/Dock 启动的 GUI 进程 PATH 不含 nvm/homebrew 的 node:dsh 定位
    /// 虽可经登录 shell 回退(`find_global_dsh`),但即便定位成功,shebang 的
    /// `env node` 解析失败仍会让 dsh 启动即死(启动日志首行
    /// `env: node: No such file or directory`),`wait_until_ready` 空转到
    /// 120s 超时,面板永远停在"启动中"。当前进程 PATH 已含 node 时(终端
    /// 启动)不覆盖,保留完整原环境。
    fn apply_shebang_path_env(cmd: &mut Command) {
        let own = std::env::var("PATH").unwrap_or_default();
        if Self::find_executable_in_path(&own, "node").is_some() {
            return;
        }
        for (key, val) in get_user_env() {
            if key == "PATH" && Self::find_executable_in_path(&val, "node").is_some() {
                log::info!("[dsh] injecting login-shell PATH (node absent from GUI PATH)");
                cmd.env("PATH", val);
                return;
            }
        }
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
        // 且包名须与 package.json 的 name 一致(client-modules 的
        // nearestPackage 按请求 specifier 严格匹配 manifest name,不一致
        // 同样静默不入表,如 name 写成无 scope 的 'zap-bridge-client')。
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
    /// 定向到本次启动的日志文件):`dsh web: http://127.0.0.1:<port>/?token=<token>`。
    /// 0.1.2-rc.1 起 web 端有 token 鉴权,裸地址 GET 返回 401,故必须解析
    /// 该 URL 用作探测与 webview 加载地址;URL 中的 token 随启动随机生成。
    ///
    /// 同时盯着子进程本身:启动期退出(插件加载失败、node 报错等)立即失败
    /// 并报出日志里的真实错误,而不是空转到 `STARTUP_TIMEOUT` 后把原因
    /// 掩盖成「就绪超时」。
    async fn wait_until_ready(child: &mut async_process::Child, log_path: &Path) -> Result<String> {
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
            // 子进程已退出:启动期崩溃,立刻失败(探测间隔内即可发现)。
            if let Ok(Some(status)) = child.try_status() {
                let log_text = std::fs::read_to_string(log_path).unwrap_or_default();
                let error = match Self::extract_fatal_error(&log_text) {
                    Some(detail) => format!("dsh web exited during startup ({status}): {detail}"),
                    None => format!("dsh web exited during startup ({status})"),
                };
                bail!("{error}");
            }

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

    /// 从 dsh 启动日志提取第一条错误行(供启动期退出时展示真实原因)。
    ///
    /// dsh 是 Node 程序:崩溃时把 `Error: ...` / `SyntaxError: ...` 连同嵌套
    /// cause 一起打到 stdout/stderr(已重定向到日志)。取**第一条**含
    /// `Error: ` 的行——它是最外层、信息量最大的那条,嵌套的重复 cause 都
    /// 在它之后;找不到(非 Node 报错形态)返回 None,调用方只报退出状态。
    fn extract_fatal_error(log_text: &str) -> Option<String> {
        /// 单行错误上限:Node 的 cause 链可能把整段堆栈塞进一行。
        const MAX_ERROR_CHARS: usize = 500;
        log_text
            .lines()
            .map(str::trim)
            .find(|line| line.contains("Error: "))
            .map(|line| line.chars().take(MAX_ERROR_CHARS).collect())
    }

    /// 崩溃放弃重启时的失败原因(供 pane 展示与复制)。
    ///
    /// 崩溃日志里通常留有针对性的报错行;拿不到时给出「无错误行 + 日志路径」,
    /// 而不是 `repeated crashes` 这类对用户与 Agent 都无信息的占位串。
    /// 日志按启动分份,这里取最近一份(就是刚崩溃那次)。
    pub fn crash_failure_reason() -> String {
        let Ok(dir) = Self::dsh_data_dir() else {
            return "dsh web crashed repeatedly".to_string();
        };
        let Some(log_path) = Self::latest_web_log_path(&dir) else {
            return format!(
                "dsh web crashed repeatedly; no {DSH_WEB_LOG_PREFIX}*.log in {}",
                dir.display()
            );
        };
        let log_text = std::fs::read_to_string(&log_path).unwrap_or_default();
        Self::crash_failure_reason_from(&log_path, &log_text)
    }

    /// [`crash_failure_reason`](Self::crash_failure_reason) 的纯函数部分(可单测)。
    fn crash_failure_reason_from(log_path: &Path, log_text: &str) -> String {
        match Self::extract_fatal_error(log_text) {
            Some(detail) => format!("dsh web crashed repeatedly: {detail}"),
            None => format!(
                "dsh web crashed repeatedly; no error line in {}",
                log_path.display()
            ),
        }
    }

    /// 从本次启动的日志读取就绪 URL(尚无输出时返回 None)。
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
        // 新一轮启动意图 = 撤销待停:否则 `open_dsh_pane`「先启动、后建 pane」
        // 之间的间隙里,上一轮遗留的待停会在下一帧把刚起的实例停掉。
        self.stop_pending = false;
        self.consecutive_crashes = 0;
        self.generation += 1;
        self.error = None;
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
        self.error = None;
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
        self.stop_pending = false;
        if let Some(mut child) = self.child.take() {
            Self::terminate_child(&mut child);
        }
        self.set_status(DshRuntimeStatus::Stopped);
        self.url = None;
        self.consecutive_crashes = 0;
        self.error = None;
    }

    /// 登记「待停止」(`DshPane::detach` 只在 `DetachType::Closed` 时调用)。
    ///
    /// 只置位,不在此刻终止子进程:原因见 `stop_pending` 字段说明。之所以
    /// 不能在 pane 的 detach 现场直接判定「还有没有别的 dsh pane」,是因为
    /// 那一刻该 PaneGroup 正处于 `update` 中、不在 `window.views` 里,遍历会
    /// **漏计**它自己(仍隐藏着 dsh pane),从而误判「无 pane」而误停进程。
    pub fn mark_stop_pending(&mut self) {
        log::info!("[dsh] mark_stop_pending (child={})", self.child.is_some());
        self.stop_pending = true;
    }

    /// 判定待停止请求(由 lib.rs 每帧调用,并在 detach 登记后即时调用一次)。
    ///
    /// `has_dsh_pane` 为 `any_dsh_pane` 的结果。只有确认一个 dsh pane 都不剩
    /// 时才真正停止:关 tab 后 pane 只是被隐藏(仍在该窗口的 `pane_contents`
    /// 中),撤销与菜单恢复还要靠它,所以那时不会停;而关窗口后该窗口不再被
    /// `any_dsh_pane` 枚举(其视图只留在 undo 栈的 `ClosedWindowData` 里),
    /// 于是关窗会直接停止——与 `c2efed09d` 确立的既有行为一致。
    ///
    /// 仍有 pane 时**保持待停**(不清除标志):这正是「关掉旧 tab 后立刻重开」
    /// 不误杀新实例的原因(标志留着,下次判定依然不会停),同时避免「先看到
    /// 尚未被回收的旧 pane、pane 随后才消失」的时序竞态被误判成取消——
    /// pane 的销毁只发生一次,清掉标志就再没有补判的机会,会永久漏停。
    pub fn poll_pending_stop(&mut self, has_dsh_pane: bool) {
        if !self.stop_pending || has_dsh_pane {
            return;
        }
        self.stop_pending = false;
        log::info!("[dsh] pending stop confirmed: no dsh pane left, stopping");
        self.request_stop();
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

/// 当前 app 内**打开窗口**里是否还存在 dsh pane。
///
/// 关 tab 后被隐藏的 pane 仍在所属窗口的 `pane_contents` 中,会被数到(撤销
/// 与菜单恢复都靠它);但**已关闭、仍可撤销恢复的窗口**里的 dsh pane 数不到
/// (其视图只留在 undo 栈的 `ClosedWindowData` 里,窗口已不在 `windows` 表)。
/// 所以它不是停止判据——判据见 `has_reachable_dsh_pane`。
///
/// 遍历方式与 `quit_warning` 中 dsh 的判定同构(沿用既有做法)。
///
/// 只能在 AppContext 级调用:在 pane 的 detach 现场内联调用会**漏计**正处于
/// `update` 中的那个 PaneGroup(`update_view` 期间它不在 `window.views` 里,
/// `views_of_type` 枚举不到),从而把「还有 pane」误判成「无 pane」。
/// 这里不会 panic,但会误停,所以登记与判定必须分开。
pub(crate) fn any_dsh_pane(ctx: &AppContext) -> bool {
    ctx.window_ids().any(|window_id| {
        ctx.views_of_type::<PaneGroup>(window_id).is_some_and(|views| {
            views
                .into_iter()
                .any(|view| view.as_ref(ctx).dsh_panes().next().is_some())
        })
    })
}

/// 是否还有「用户能再拿回来」的 dsh pane —— **停止判据统一用它**。
///
/// = 打开窗口里的 dsh pane(`any_dsh_pane`) ∪ undo 栈里仍有可恢复窗口。
///
/// 后者必须算上:关窗口在宽限期内可 ⌘⇧T 撤销恢复,其 dsh pane 随之复活,而
/// `any_dsh_pane` 看不到它。只看 `any_dsh_pane` 会在「先关的窗口撤销项过期时」
/// 或「关窗口后的那一帧」把「已无 pane」当成事实,提前杀掉子进程,恢复出来就是
/// 死会话。窗口条目无法逐个探测是否含 dsh pane,故按「还有窗口可恢复」保守
/// 保留——代价只是多留一会儿,不会泄漏。
///
/// **只能在 `UndoCloseStack` 的 update 之外调用**(内部会读该单例,重入读会
/// panic);栈内部改用 `stop_if_no_dsh_pane` 并显式传入该布尔值。
pub(crate) fn has_reachable_dsh_pane(ctx: &AppContext) -> bool {
    has_reachable_dsh_pane_with(ctx, UndoCloseStack::as_ref(ctx).has_restorable_window())
}

/// [`has_reachable_dsh_pane`] 的参数化形式:「undo 栈里还有可恢复窗口」由调用方
/// 给出,供无法回读 `UndoCloseStack` 的场合(栈自己的 update 内)使用。
pub(crate) fn has_reachable_dsh_pane_with(
    ctx: &AppContext,
    has_restorable_window: bool,
) -> bool {
    any_dsh_pane(ctx) || has_restorable_window
}

/// 登记一次「待停止」并立即判定一次(供「窗口确认不恢复」的兜底路径调用)。
///
/// 关窗口只产生 `HiddenForClose`,且此后不会再产生 `Closed`(见 lib.rs 窗口关闭
/// 处的说明);而 `HiddenForClose` 不登记待停(为了让撤销恢复能连回活进程)。
/// 因此必须在窗口的 undo 条目真正过期时补登记,否则子进程会一直活到 app 退出。
///
/// `has_restorable_window` 由调用方给出:本函数会被 `UndoCloseStack` 内部调用
/// (窗口条目过期时),那里回读该单例会 panic。
pub(crate) fn stop_if_no_dsh_pane(ctx: &mut AppContext, has_restorable_window: bool) {
    // `DshRuntime` 只在 DshPane flag 打开时注册为单例,未打开时 handle 会 panic。
    if !FeatureFlag::DshPane.is_enabled() {
        return;
    }
    let has_dsh_pane = has_reachable_dsh_pane_with(ctx, has_restorable_window);
    DshRuntime::handle(ctx).update(ctx, |runtime, _ctx| {
        runtime.mark_stop_pending();
        runtime.poll_pending_stop(has_dsh_pane);
    });
}

impl Entity for DshRuntime {
    type Event = super::bridge::BridgeEvent;
}

impl SingletonEntity for DshRuntime {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 触碰全局隐私开关的用例须串行:`DshRuntime::new()` 会按磁盘设置重写
    /// `TERMINAL_CONTEXT_ENABLED`(本机 dsh_settings.json 为 true 时置 true),
    /// 并行执行会把 [`terminal_context_privacy_gate`] 断言的中间态踩掉
    /// (实测约 1/3 概率失败)。
    static PRIVACY_STATIC_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 构造 `DshRuntime` 的用例统一入口:构造期间持锁,与隐私开关用例互斥。
    fn test_runtime() -> DshRuntime {
        let _guard = PRIVACY_STATIC_TEST_LOCK.lock();
        DshRuntime::new()
    }

    /// PATH 查找:命中首个含可执行 `dsh` 的目录;无可执行时返回 None。
    /// 仅 unix:is_executable_file 的可执行判定按 unix 权限位实现。
    #[cfg(unix)]
    #[test]
    fn find_dsh_in_path_hits_first_executable() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("zap-dsh-test-{}", std::process::id()));
        let dir_a = root.join("a");
        let dir_b = root.join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let dsh = dir_b.join("dsh");
        std::fs::write(&dsh, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&dsh, std::fs::Permissions::from_mode(0o755)).unwrap();

        let path_env = format!("{}:{}", dir_a.display(), dir_b.display());
        assert_eq!(DshRuntime::find_dsh_in_path(&path_env), Some(dsh.clone()));

        std::fs::remove_file(&dsh).unwrap();
        assert_eq!(DshRuntime::find_dsh_in_path(&path_env), None);

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 通用可执行查找(参数化名称):dsh shebang 修复用它探测 PATH 中的
    /// `node`;同源逻辑命中首个含该可执行文件的目录。
    #[cfg(unix)]
    #[test]
    fn find_executable_in_path_by_name() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("zap-dsh-node-{}", std::process::id()));
        let dir_a = root.join("a");
        let dir_b = root.join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let node = dir_b.join("node");
        std::fs::write(&node, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();

        let path_env = format!("{}:{}", dir_a.display(), dir_b.display());
        assert_eq!(
            DshRuntime::find_executable_in_path(&path_env, "node"),
            Some(node.clone())
        );
        assert_eq!(DshRuntime::find_executable_in_path(&path_env, "dsh"), None);

        std::fs::remove_dir_all(&root).unwrap();
    }

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

    /// 渠道筛选:各渠道独立与已装版本比对,顺序保持 UPDATE_CHANNELS;
    /// 落后于该渠道(含同版)的渠道不提示。
    #[test]
    fn channel_updates_filtering() {
        let tags = vec![
            ("latest", "0.1.5-rc.1".to_string()),
            ("next", "0.1.5-rc.2".to_string()),
            ("alpha", "0.1.6-alpha.1".to_string()),
        ];
        // 落后于全部渠道 → 三条都提示。
        let got = channel_updates("0.1.4", &tags);
        assert_eq!(
            got,
            vec![
                ("latest", "0.1.5-rc.1".to_string()),
                ("next", "0.1.5-rc.2".to_string()),
                ("alpha", "0.1.6-alpha.1".to_string()),
            ]
        );
        // 已装 latest 同版 → 只提示 next/alpha(预览渠道)。
        let got = channel_updates("0.1.5-rc.1", &tags);
        assert_eq!(
            got,
            vec![
                ("next", "0.1.5-rc.2".to_string()),
                ("alpha", "0.1.6-alpha.1".to_string()),
            ]
        );
        // 全程已是最新 → 空。
        assert!(channel_updates("0.1.6-alpha.1", &tags).is_empty());
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

    /// 待停判定:无 dsh pane 才真正停止;仍有 pane(含 undo 宽限期内隐藏的
    /// pane)则保持待停、既不停也不清除标志。这是「关掉旧 tab 后立刻重开不再
    /// 误杀新实例」的核心状态机。
    #[test]
    fn poll_pending_stop_respects_live_pane() {
        let mut runtime = DshRuntime::new();

        // 未登记待停:任何输入都不动作。
        runtime.set_status(DshRuntimeStatus::Ready);
        runtime.poll_pending_stop(false);
        assert_eq!(runtime.status(), DshRuntimeStatus::Ready);
        assert!(!runtime.stop_pending);

        // 登记待停但仍有 pane:保持待停(不清除),不停止。
        runtime.mark_stop_pending();
        assert!(runtime.stop_pending);
        runtime.poll_pending_stop(true);
        assert!(
            runtime.stop_pending,
            "a live dsh pane must keep the pending stop armed, not clear it"
        );
        assert_eq!(runtime.status(), DshRuntimeStatus::Ready);

        // pane 全部消失后才真正停止(标志可能是更早一轮遗留的)。
        runtime.poll_pending_stop(false);
        assert!(!runtime.stop_pending);
        assert_eq!(runtime.status(), DshRuntimeStatus::Stopped);
    }

    /// 新一轮启动清掉待停意图:否则 open_dsh_pane「先启动、后建 pane」的
    /// 间隙里,上一轮遗留的待停会在下一帧停掉刚起的实例。
    #[test]
    fn begin_start_clears_pending_stop() {
        let mut runtime = DshRuntime::new();
        runtime.mark_stop_pending();
        runtime.begin_start();
        assert!(
            !runtime.stop_pending,
            "begin_start must clear the pending stop"
        );
        assert_eq!(runtime.status(), DshRuntimeStatus::Starting);
    }

    /// 主动停止同样清掉待停意图,避免停止后残留标记。
    #[test]
    fn request_stop_clears_pending_stop() {
        let mut runtime = DshRuntime::new();
        runtime.mark_stop_pending();
        runtime.request_stop();
        assert!(!runtime.stop_pending);
        assert_eq!(runtime.status(), DshRuntimeStatus::Stopped);
    }

    /// 终端上下文隐私开关:关闭时返回空,开启后返回暂存命令。
    #[test]
    fn terminal_context_privacy_gate() {
        // 全程持锁:并行用例构造 DshRuntime 会按磁盘设置重写该全局静态。
        let _guard = PRIVACY_STATIC_TEST_LOCK.lock();
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
            let mut runtime = test_runtime();
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
            let mut runtime = test_runtime();
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
            let mut runtime = test_runtime();
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

    /// 失败原因记录:set_failed 同时写入错误串与 Failed 状态;重新启动
    /// 或主动停止时清空,避免展示过期错误。
    #[test]
    fn failed_error_is_stored_and_cleared() {
        let mut runtime = test_runtime();
        assert!(runtime.error().is_none());

        runtime.set_failed("dsh command not found in PATH".to_string());
        assert_eq!(runtime.status(), DshRuntimeStatus::Failed);
        assert_eq!(runtime.error(), Some("dsh command not found in PATH"));

        runtime.begin_start();
        assert!(runtime.error().is_none(), "begin_start clears stale error");

        runtime.set_failed("dsh web did not become ready".to_string());
        runtime.request_stop();
        assert!(runtime.error().is_none(), "request_stop clears error");
    }

    /// 启动期退出:错误提取取日志里第一条 `Error: ` 行(最外层原因),
    /// 忽略它之前的正常输出与之后的嵌套 cause;无错误行时返回 None。
    #[test]
    fn extract_fatal_error_takes_first_error_line() {
        let sample = "[dsh-wechat] session/event listener attached\n\
                      [opencode-session-id] mounted: providers=[opencode]\n\
                      throw new Error(`${binName}: ${stage}: ${detail}`)\n\
                      \n\
                      Error: dsh: plugin tree failed to load: failed to import dsh-rewind-plugin\n\
                      \x20   at #asyncInstantiate (node:internal/modules/esm/module_job:455:21)\n\
                      SyntaxError: The requested module '@deepseek-ai/dsh-session' does not provide ...\n";
        assert_eq!(
            DshRuntime::extract_fatal_error(sample).as_deref(),
            Some("Error: dsh: plugin tree failed to load: failed to import dsh-rewind-plugin")
        );

        // 无 Node 报错形态(如正常启动输出)时不给出原因。
        assert_eq!(
            DshRuntime::extract_fatal_error("dsh web: http://127.0.0.1:8080/?token=abc\n"),
            None
        );
    }

    /// 启动期退出:wait_until_ready 立即失败(不等 STARTUP_TIMEOUT),错误里
    /// 带上日志中的真实原因,而不是「就绪超时」。
    #[cfg(unix)]
    #[test]
    fn wait_until_ready_fails_fast_on_early_exit() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let dir = std::env::temp_dir().join(format!("zap-dsh-early-exit-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let log_path = dir.join("zap-dsh-web.log");
            std::fs::write(
                &log_path,
                "[plugin] mounted\n\
                 Error: dsh: plugin tree failed to load: boom\n\
                 \x20   at #asyncInstantiate (node:internal/modules/esm/module_job:455:21)\n",
            )
            .unwrap();

            // 立刻退出的假 dsh 子进程(模拟插件加载失败后 node 退出)。
            let mut cmd = command::r#async::Command::new("sh");
            cmd.arg("-c").arg("exit 1");
            let mut child = cmd.spawn().expect("spawn fake dsh");

            let started = std::time::Instant::now();
            let err = DshRuntime::wait_until_ready(&mut child, &log_path)
                .await
                .expect_err("early exit must fail");
            let message = format!("{err:#}");
            assert!(message.contains("exited during startup"), "{message}");
            assert!(
                message.contains("Error: dsh: plugin tree failed to load: boom"),
                "{message}"
            );
            // 一个探测周期内就发现,远早于 120s 超时。
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "took {:?}",
                started.elapsed()
            );

            std::fs::remove_dir_all(&dir).ok();
        });
    }

    /// 崩溃放弃重启:原因取日志里的首条错误行;日志里没有错误行时回退到
    /// 「无错误行 + 日志路径」,而不是占位串。
    #[test]
    fn crash_failure_reason_prefers_log_error_line() {
        let log_path = Path::new("/tmp/zap-dsh-web.log");
        assert_eq!(
            DshRuntime::crash_failure_reason_from(log_path, "[plugin] ok\nError: boom\n"),
            "dsh web crashed repeatedly: Error: boom"
        );
        assert_eq!(
            DshRuntime::crash_failure_reason_from(log_path, "[plugin] ok\n"),
            "dsh web crashed repeatedly; no error line in /tmp/zap-dsh-web.log"
        );
    }

    /// `dsh.pid` 记录的解析与「该不该回收」判定:只有记录可解析、上次 Zap 已死、
    /// 且不是本进程自己的记录时,才给出要回收的 pgid。
    #[cfg(unix)]
    #[test]
    fn orphan_group_decision_from_pid_record() {
        // 上次 Zap 早已不存在(取值超出 pid 上限)→ 回收它留下的组。
        let dead = i32::MAX - 1;
        assert_eq!(
            DshRuntime::orphan_group_from_record(&format!(
                "zap_pid={dead}\npgid=4321\nstarted_at=1\n"
            )),
            Some(4321)
        );

        // 本进程自己的记录(崩溃重启路径):不回收。
        let own = format!("zap_pid={}\npgid=4321\nstarted_at=1\n", std::process::id());
        assert_eq!(DshRuntime::orphan_group_from_record(&own), None);

        // 另一个仍活着的进程(pid 1 恒存在且不属于本用户 → EPERM 视为活着):
        // 多实例并存时不能误杀另一个 Zap 的 dsh。
        assert_eq!(
            DshRuntime::orphan_group_from_record("zap_pid=1\npgid=4321\nstarted_at=1\n"),
            None
        );
        assert!(DshRuntime::process_alive(1));

        // 旧格式(裸 pid)与残缺/非法记录:解析不出就不回收,不赌 pid 复用。
        assert_eq!(DshRuntime::orphan_group_from_record("69209"), None);
        assert_eq!(DshRuntime::orphan_group_from_record("zap_pid=1\n"), None);
        assert_eq!(
            DshRuntime::orphan_group_from_record("zap_pid=1\npgid=0\n"),
            None
        );
        assert_eq!(
            DshRuntime::orphan_group_from_record("zap_pid=1\npgid=not-a-pid\n"),
            None
        );
    }

    /// 日志分份:只保留最近 DSH_WEB_LOG_KEEP 份,本次启动那份永不删除
    /// (同秒重启的带序号文件按路径排在最前,会被误当最旧),目录里的无关文件
    /// (如 dsh.pid)也不能删。
    #[test]
    fn prune_web_logs_keeps_newest() {
        let dir = std::env::temp_dir().join(format!("zap-dsh-logs-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        let total = DSH_WEB_LOG_KEEP + 3;
        for index in 0..total {
            std::fs::write(dir.join(format!("{DSH_WEB_LOG_PREFIX}{index}.log")), b"x").unwrap();
        }
        // 本次启动那份:与最新一份同秒(带序号),路径比较排在 `<秒>.log` 之前。
        let current = dir.join(format!("{DSH_WEB_LOG_PREFIX}{}-2.log", total - 1));
        std::fs::write(&current, b"x").unwrap();
        std::fs::write(dir.join(DSH_PID_FILE), b"zap_pid=1\n").unwrap();

        DshRuntime::prune_web_logs(&dir, &current);

        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| is_web_log_name(name))
            .collect();
        names.sort();
        assert_eq!(names.len(), DSH_WEB_LOG_KEEP);
        assert!(current.exists(), "本次启动的日志不能被删");
        assert!(dir.join(DSH_PID_FILE).exists(), "无关文件不能被删");
        // 保留 current + 最新的 KEEP-1 份旧日志,更旧的那几份被删。
        let removed = total - DSH_WEB_LOG_KEEP + 1;
        for index in 0..removed {
            assert!(
                !dir.join(format!("{DSH_WEB_LOG_PREFIX}{index}.log")).exists(),
                "最旧的 {index} 应被删除"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 停止按**整组**回收:dsh 派生的子进程(同组)在组长退出后也必须一起死,
    /// 否则每停一次 dsh 就留一批孤儿。
    #[cfg(unix)]
    #[test]
    fn terminate_child_kills_whole_process_group() {
        let dir = std::env::temp_dir().join(format!("zap-dsh-group-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("grandchild.pid");

        // 组长(模拟 dsh)在自成一组的组里再派生一个长睡进程,并把它的 pid 落盘。
        let mut cmd = Command::new_with_process_group("sh");
        cmd.arg("-c")
            .arg(format!("sleep 300 & echo $! > {}; wait", pid_file.display()));
        let mut child = cmd.spawn().expect("spawn fake dsh group");
        let pgid = child.id();

        let mut grandchild = None;
        for _ in 0..200 {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
            {
                grandchild = Some(pid);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let grandchild = grandchild.expect("grandchild pid should be written");

        DshRuntime::terminate_child(&mut child);

        assert!(
            !DshRuntime::process_alive(pgid as i32),
            "group leader {pgid} should be gone"
        );
        // 孙进程被 reparent 后由 launchd 回收,给一点时间避免读到僵尸态。
        for _ in 0..100 {
            if !DshRuntime::process_alive(grandchild) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !DshRuntime::process_alive(grandchild),
            "grandchild {grandchild} survived group termination"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 启动回收:记录里的上次 Zap 已死时整组回收遗留进程;记录属于本进程
    /// (崩溃重启路径)时不动它。这里刻意让组长先退出、只剩它派生的进程活着,
    /// 复现真实残留形态(上次 Zap 被杀后留下的 dsh)。
    #[cfg(unix)]
    #[test]
    fn reclaim_orphan_group_kills_leftover_group() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let dir = std::env::temp_dir().join(format!("zap-dsh-reclaim-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();

            // 组长自成一组的组里派生一个长睡进程后立即退出。
            let mut cmd = Command::new_with_process_group("sh");
            cmd.arg("-c").arg("sleep 300 & exit 0");
            let mut child = cmd.spawn().expect("spawn leftover group");
            let pgid = child.id();
            for _ in 0..200 {
                if child.try_status().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                child.try_status().ok().flatten().is_some(),
                "leader should have exited"
            );
            assert!(
                DshRuntime::process_group_alive(pgid as i32),
                "derived process should keep the group alive"
            );

            // 记录属于本进程(崩溃重启路径):不回收。
            std::fs::write(
                dir.join(DSH_PID_FILE),
                format!("zap_pid={}\npgid={pgid}\nstarted_at=1\n", std::process::id()),
            )
            .unwrap();
            DshRuntime::reclaim_orphan_group(&dir).await;
            assert!(
                DshRuntime::process_group_alive(pgid as i32),
                "own record must not be reclaimed"
            );

            // 记录属于已死的上次 Zap:整组回收。
            std::fs::write(
                dir.join(DSH_PID_FILE),
                format!("zap_pid={}\npgid={pgid}\nstarted_at=1\n", i32::MAX - 1),
            )
            .unwrap();
            DshRuntime::reclaim_orphan_group(&dir).await;
            assert!(
                !DshRuntime::process_group_alive(pgid as i32),
                "leftover group must be reclaimed"
            );

            std::fs::remove_dir_all(&dir).ok();
        });
    }

    /// begin_start 复位 stopping 与崩溃计数,并置位 Starting。
    #[test]
    fn begin_start_resets_state() {
        let mut runtime = test_runtime();
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
            let mut runtime = test_runtime();
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
            let mut runtime = test_runtime();

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
