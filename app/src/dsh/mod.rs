//! DeepSeek Harness (dsh) 集成:管理 dsh runtime 子进程,提供 Web UI pane。
//!
//! 架构:
//! - `runtime`:DshRuntime 单例,负责定位全局 `dsh` 命令并管理 `dsh web` 子进程的
//!   启动/停止/崩溃重启,以及就绪状态(HTTP probe)管理。
//! - `pane`:复用 `browser::BrowserPane` 渲染 dsh Web UI(不新建 pane 类型)。
//!
//! 入口:WorkspaceAction::OpenDshPane → DshRuntime 启动 → 就绪后打开 BrowserPane。

pub(crate) mod bridge;
pub(crate) mod pane;
pub(crate) mod runtime;

pub use pane::DshPane;
pub use runtime::{
    DshRestartResult, DshRuntime, DshRuntimeStatus, DshStartResult, DshUpdateCheck, PollResult,
};
