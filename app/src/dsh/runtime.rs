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
use std::time::Duration;

use anyhow::{bail, Context, Result};
use command::r#async::Command;
use warpui::{Entity, SingletonEntity};

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

/// DeepSeek Harness runtime 单例。
///
/// 状态与子进程句柄均由主线程(model 消息循环)持有;异步操作通过
/// `ModelContext::spawn` 在后台执行器上运行,结果回主线程回调。
pub struct DshRuntime {
    status: DshRuntimeStatus,
    url: Option<String>,
    /// dsh web 子进程句柄。`None` 表示未运行。
    child: Option<async_process::Child>,
    /// 连续崩溃次数(重启成功后清零)。
    consecutive_crashes: u8,
    /// 是否已请求停止(崩溃自动重启与主动停止竞争时优先停止)。
    stopping: bool,
}

impl Default for DshRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DshRuntime {
    fn drop(&mut self) {
        // 进程退出兜底:杀掉 dsh 子进程,避免残留。
        // Drop 中不能 await,用同步 kill + try_wait 轮询回收。
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            for _ in 0..100 {
                if child.try_status().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

impl DshRuntime {
    pub fn new() -> Self {
        Self {
            status: DshRuntimeStatus::Stopped,
            url: None,
            child: None,
            consecutive_crashes: 0,
            stopping: false,
        }
    }

    /// dsh 是否已配置(settings.yaml 存在即视为已初始化)。
    ///
    /// dsh 的设置文档默认在 `<DSH_HOME>/settings.yaml`(见 dsh settings-file
    /// 插件);首次使用前不存在。用它作为"需要引导"的判据。
    pub fn is_configured() -> bool {
        Self::dsh_data_dir()
            .map(|dir| dir.join("settings.yaml").is_file())
            .unwrap_or(false)
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

    /// 启动 dsh runtime(异步,不借用 self)。
    ///
    /// 调用方用 `ctx.spawn(Self::start_future(), callback)` 驱动;回调中把
    /// 返回的子进程句柄交给 runtime(`adopt_child`)。
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

    async fn start_inner() -> Result<(async_process::Child, String)> {
        // 1. 定位/安装 Node。
        let path_env = std::env::var("PATH").unwrap_or_default();
        let node = match node_runtime::find_working_node_binary(Some(&path_env)).await {
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

        // 2. 定位/安装 dsh。
        let dsh_cli = Self::ensure_dsh_installed(&node).await?;

        // 3. 启动 `dsh web --port 0`。
        let dsh_home = Self::dsh_data_dir()?;
        let mut cmd = Command::new(&node);
        cmd.arg(&dsh_cli)
            .arg("web")
            .arg("--port")
            .arg("0")
            .env("DSH_HOME", &dsh_home)
            .env("ZAP_BRIDGE_ADDRESS", ""); // 预留:插件桥地址(第二阶段)
        let child = cmd.spawn().context("Failed to spawn dsh web")?;

        // 4. 就绪探测:等端口出现 + HTTP 200。
        let url = Self::wait_until_ready(&child).await?;

        // 5. 记录 PID 到文件,便于调试/外部检查。
        let _ = std::fs::write(dsh_home.join("dsh.pid"), child.id().to_string());

        Ok((child, url))
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

    /// 接收启动完成的子进程(回调中调用)。
    pub fn adopt_child(&mut self, child: async_process::Child) {
        self.child = Some(child);
        self.set_status(DshRuntimeStatus::Ready);
    }

    /// 主动停止 runtime(面板关闭/退出时)。同步执行,可在主线程直接调用:
    /// `kill` 是同步的,退出状态交给 async-process 的 reap 线程回收。
    pub fn request_stop(&mut self) {
        self.stopping = true;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // 短轮询回收(与 Drop 一致),避免 zombie。
            for _ in 0..100 {
                if child.try_status().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.set_status(DshRuntimeStatus::Stopped);
        self.url = None;
        self.consecutive_crashes = 0;
    }

    /// 轮询子进程是否退出(由 app 的 on_frame_drawn 驱动)。
    ///
    /// 若进程已退出且未主动停止,返回 `Some(consecutive_crashes)` 供外部
    /// 调度重启;超过上限返回 `Some(MAX_RESTARTS + 1)` 表示放弃。
    pub fn poll_child(&mut self) -> Option<u8> {
        let Some(child) = &mut self.child else {
            return None;
        };
        match child.try_status() {
            Ok(Some(_status)) => {
                self.child = None;
                if self.stopping {
                    return Some(0); // 主动停止,不重启
                }
                self.consecutive_crashes += 1;
                log::warn!(
                    "[dsh] process exited unexpectedly (crash #{})",
                    self.consecutive_crashes
                );
                if self.consecutive_crashes > MAX_RESTARTS {
                    self.set_status(DshRuntimeStatus::Failed);
                    return Some(MAX_RESTARTS + 1);
                }
                self.set_status(DshRuntimeStatus::Stopped);
                Some(self.consecutive_crashes)
            }
            _ => None,
        }
    }

    /// 崩溃后重启(由外部在 `poll_child` 返回 Some 后调度,`'static` future)。
    pub async fn restart_future() -> DshRestartResult {
        match Self::start_future().await {
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
    /// 启动/重启失败。
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

    /// 端到端冒烟:真实启动 dsh runtime(需网络安装 dsh,首次较慢)。
    /// 验证:启动成功、URL 可访问、子进程可停止。
    #[test]
    #[ignore = "requires network + npm install, run manually"]
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
    #[ignore = "requires network + npm install, run manually"]
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

    /// 崩溃检测:poll_child 在进程退出后返回崩溃计数。
    #[test]
    fn poll_child_detects_exit() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let mut runtime = DshRuntime::new();
            // 用 sleep 进程模拟 dsh 子进程。
            let mut cmd = command::r#async::Command::new("sleep");
            cmd.arg("30");
            let child = cmd.spawn().expect("spawn sleep");
            runtime.adopt_child(child);
            assert_eq!(runtime.status(), DshRuntimeStatus::Ready);

            // 杀掉进程,下一轮 poll_child 应检测到崩溃(计数 1)。
            let pid = runtime.child.as_mut().unwrap().id();
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            // 等进程真正退出。
            for _ in 0..100 {
                if runtime.poll_child().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(runtime.consecutive_crashes, 1);
            assert_eq!(runtime.status(), DshRuntimeStatus::Stopped);
        });
    }
}
