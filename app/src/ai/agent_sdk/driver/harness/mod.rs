use std::collections::HashMap;
use std::ffi::OsString;

use warp_cli::agent::Harness;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::util::path::resolve_executable;
use warp_cli::{OZ_CLI_ENV, OZ_HARNESS_ENV, OZ_PARENT_RUN_ID_ENV, OZ_RUN_ID_ENV};
use warp_core::channel::ChannelState;

use super::{
    AgentDriverError, LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV,
    LEGACY_OZ_PARENT_STATE_ROOT_ENV, OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV,
    OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
};

/// Check that `cli` is installed and on PATH, returning a `HarnessSetupFailed`
/// error with an optional install-docs link when it isn't.
///
/// Resolution is consistent with the CLI-agent install scan
/// ([`crate::terminal::cli_agent::cli_agent_search_dirs`]): when the process
/// `PATH` misses the binary, common install dirs are probed as a fallback.
/// macOS GUI apps launched from Finder/Dock inherit launchd's short `PATH`,
/// so a Homebrew-installed CLI would otherwise show as installed in settings
/// but fail here at launch time (issue #253).
pub(crate) fn validate_cli_installed(
    cli: &str,
    install_docs_url: Option<&str>,
) -> Result<(), AgentDriverError> {
    if resolve_cli_command(cli).is_none() {
        let mut reason = format!("'{cli}' CLI not found on your machine.");
        if let Some(url) = install_docs_url {
            reason.push_str(&format!(" Install it first: {url}"));
        }
        return Err(AgentDriverError::HarnessSetupFailed {
            harness: cli.into(),
            reason,
        });
    }
    Ok(())
}

/// Resolves a CLI command against the process `PATH`, falling back to the
/// common install dirs probed by the CLI-agent install scan.
fn resolve_cli_command(cli: &str) -> Option<std::path::PathBuf> {
    if let Some(resolved) = resolve_executable(cli) {
        return Some(resolved.into_owned());
    }
    #[cfg(unix)]
    {
        crate::terminal::cli_agent::cli_agent_search_dirs()
            .map(|dir| dir.join(cli))
            .find(|path| path.is_file())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn insert_non_empty_task_env_var(
    env_vars: &mut HashMap<OsString, OsString>,
    key: &'static str,
    value: String,
) {
    if value.is_empty() {
        return;
    }

    env_vars.insert(OsString::from(key), OsString::from(value));
}

fn insert_task_env_var_aliases(
    env_vars: &mut HashMap<OsString, OsString>,
    keys: &[&'static str],
    value: &str,
) {
    for key in keys {
        env_vars.insert(OsString::from(key), OsString::from(value));
    }
}

fn message_listener_state_root() -> Option<String> {
    [
        OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
        LEGACY_OZ_PARENT_STATE_ROOT_ENV,
    ]
    .into_iter()
    .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
}

fn task_env_vars_for_harness_name(
    task_id: Option<&AmbientAgentTaskId>,
    parent_run_id: Option<&str>,
    selected_harness: Harness,
) -> HashMap<OsString, OsString> {
    let mut env_vars = HashMap::with_capacity(7);

    if let Some(id) = task_id {
        env_vars.insert(
            OsString::from(OZ_RUN_ID_ENV),
            OsString::from(id.to_string()),
        );
    }

    if let Some(parent_run_id) = parent_run_id.filter(|id| !id.is_empty()) {
        env_vars.insert(
            OsString::from(OZ_PARENT_RUN_ID_ENV),
            OsString::from(parent_run_id),
        );
    }

    env_vars.insert(
        OsString::from(OZ_CLI_ENV),
        OsString::from(
            std::env::current_exe()
                .unwrap_or_else(|_| ChannelState::channel().cli_command_name().into()),
        ),
    );
    // `OZ_HARNESS` is consumed by child-agent telemetry when the child CLI emits
    // `run message *` events.
    env_vars.insert(
        OsString::from(OZ_HARNESS_ENV),
        OsString::from(selected_harness.to_string()),
    );
    if selected_harness == Harness::Claude && task_id.is_some() {
        insert_task_env_var_aliases(
            &mut env_vars,
            &[
                OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV,
                LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV,
            ],
            "1",
        );
        if let Some(state_root) = message_listener_state_root() {
            insert_task_env_var_aliases(
                &mut env_vars,
                &[
                    OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
                    LEGACY_OZ_PARENT_STATE_ROOT_ENV,
                ],
                &state_root,
            );
        }
    }
    // Server URL overrides are disabled on release channels, so there's no
    // override to propagate to child processes there.
    env_vars
}

pub(crate) fn task_env_vars(
    task_id: Option<&AmbientAgentTaskId>,
    parent_run_id: Option<&str>,
    selected_harness: Harness,
) -> HashMap<OsString, OsString> {
    task_env_vars_for_harness_name(task_id, parent_run_id, selected_harness)
}



#[cfg(test)]
#[path = "harness_tests.rs"]
mod tests;
