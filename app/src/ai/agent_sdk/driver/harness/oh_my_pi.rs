use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use parking_lot::Mutex;
use warp_cli::agent::Harness;
use warpui::{ModelHandle, ModelSpawner};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_events::AgentEventStreamClient;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::terminal::model::block::BlockId;
use crate::terminal::CLIAgent;

use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::AgentDriverError;
use super::{HarnessRunner, SavePoint, ThirdPartyHarness};

pub(crate) struct OhMyPiHarness;

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ThirdPartyHarness for OhMyPiHarness {
    fn harness(&self) -> Harness {
        Harness::OhMyPi
    }

    fn cli_agent(&self) -> CLIAgent {
        CLIAgent::OhMyPi
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        None
    }

    fn build_runner(
        &self,
        prompt: &str,
        _system_prompt: Option<&str>,
        resumption_prompt: Option<&str>,
        _working_dir: &Path,
        _task_id: Option<AmbientAgentTaskId>,
        _agent_event_stream_client: Arc<dyn AgentEventStreamClient>,
        terminal_driver: ModelHandle<TerminalDriver>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        // Append resumption preamble to the prompt, matching other harnesses.
        let owned_prompt = match resumption_prompt {
            Some(p) if !p.is_empty() => format!("{p}\n\n{prompt}"),
            _ => prompt.to_string(),
        };
        Ok(Box::new(OhMyPiHarnessRunner::new(
            self.cli_agent().command_prefix(),
            &owned_prompt,
            terminal_driver,
        )?))
    }
}

enum OmpRunnerState {
    /// Runner is built but [`HarnessRunner::start`] has not been called yet.
    Preexec,
    /// The harness command is running (or has finished).
    Running {
        conversation_id: AIConversationId,
        block_id: BlockId,
    },
}

struct OhMyPiHarnessRunner {
    /// Full shell command: `omp --mode=rpc`
    command: String,
    /// NDJSON user_message payload, written to PTY after start.
    prompt_ndjson: String,
    terminal_driver: ModelHandle<TerminalDriver>,
    state: Mutex<OmpRunnerState>,
}

impl OhMyPiHarnessRunner {
    fn new(
        cli_command: &str,
        prompt: &str,
        terminal_driver: ModelHandle<TerminalDriver>,
    ) -> Result<Self, AgentDriverError> {
        Ok(Self {
            command: format!("{cli_command} --mode=rpc"),
            prompt_ndjson: serde_json::json!({
                "type": "user_message",
                "content": prompt,
            })
            .to_string(),
            terminal_driver,
            state: Mutex::new(OmpRunnerState::Preexec),
        })
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for OhMyPiHarnessRunner {
    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<CommandHandle, AgentDriverError> {
        let conversation_id = AIConversationId::new();
        log::info!("Created local OMP conversation {conversation_id}");

        // 1. Execute `omp --mode=rpc` in the terminal
        let command = self.command.clone();
        let terminal_driver = self.terminal_driver.clone();
        let command_handle = foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| {
                    driver.execute_command(&command, ctx)
                })
            })
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?
            .await?;

        // 2. Write the NDJSON prompt to the PTY directly, bypassing BracketedPaste
        let prompt = self.prompt_ndjson.clone();
        let td = self.terminal_driver.clone();
        foreground
            .spawn(move |_, ctx| {
                td.update(ctx, |driver, ctx| {
                    let bytes = format!("{}\n", prompt).into_bytes();
                    driver.write_to_pty(bytes, ctx);
                });
            })
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;

        *self.state.lock() = OmpRunnerState::Running {
            conversation_id,
            block_id: command_handle.block_id().clone(),
        };

        Ok(command_handle)
    }

    async fn exit(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        // Send cancel NDJSON message to omp
        let cancel_msg = r#"{"type":"cancel","reason":"user_exit"}"#.to_string();
        let td = self.terminal_driver.clone();
        foreground
            .spawn(move |_, ctx| {
                td.update(ctx, |driver, ctx| {
                    let bytes = format!("{}\n", cancel_msg).into_bytes();
                    driver.write_to_pty(bytes, ctx);
                });
            })
            .await
            .map_err(|_| anyhow::anyhow!("Agent driver dropped while sending cancel"))
    }

    async fn save_conversation(
        &self,
        _save_point: SavePoint,
        _foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        // Minimal implementation: no-op for now.
        Ok(())
    }
}
