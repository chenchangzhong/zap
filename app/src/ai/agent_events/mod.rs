//! Shared agent-event stream utilities used by third-party harness bridges.

use anyhow::{anyhow, Result};
use async_trait::async_trait;

mod driver;
mod message_hydrator;

#[cfg(test)]
pub(crate) use driver::{
    agent_event_backoff, agent_event_failures_exceeded_threshold, AgentEventDriverState,
    DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG, DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS,
    DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
};
// Zap 保留的 harness-bridge 接口重导出:非 test 构建暂无消费者(测试经 super::*
// 直接访问 driver 内部项),但该重导出边同时是这些项在非 test 构建的存活依据,
// 删除会级联 dead_code,故以 allow 压制 unused 警告而非删除。
#[allow(unused_imports)]
pub(crate) use driver::{
    run_agent_event_driver, AgentEventConsumer, AgentEventConsumerControlFlow,
    AgentEventDriverConfig, AgentEventSource, AgentEventSourceItem,
};
#[allow(unused_imports)]
pub(crate) use message_hydrator::MessageHydrator;

/// 本地 agent 事件流入口。Zap 保留接口以支持本地 driver 注入,默认实现禁用云端 RTC。
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
pub(crate) trait AgentEventStreamClient: 'static + Send + Sync {
    async fn stream_agent_events(
        &self,
        run_ids: &[String],
        since_sequence: i64,
    ) -> Result<http_client::EventSourceStream>;
}

pub(crate) struct DisabledAgentEventStreamClient;

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventStreamClient for DisabledAgentEventStreamClient {
    async fn stream_agent_events(
        &self,
        _run_ids: &[String],
        _since_sequence: i64,
    ) -> Result<http_client::EventSourceStream> {
        Err(anyhow!(
            "Agent event stream disabled in Zap - RTC endpoint is removed"
        ))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct AgentRunEvent {
    pub event_type: String,
    pub run_id: String,
    pub ref_id: Option<String>,
    pub execution_id: Option<String>,
    pub occurred_at: String,
    pub sequence: i64,
}

#[cfg(test)]
mod driver_tests;
#[cfg(test)]
mod message_hydrator_tests;
