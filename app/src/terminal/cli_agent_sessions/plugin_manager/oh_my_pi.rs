use std::sync::LazyLock;

use async_trait::async_trait;

use super::{CliAgentPluginManager, PluginInstructionStep, PluginInstructions};

pub(super) struct OhMyPiPluginManager;

#[async_trait]
impl CliAgentPluginManager for OhMyPiPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        "0.0.0"
    }

    fn can_auto_install(&self) -> bool {
        false
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &EMPTY_INSTRUCTIONS
    }

    fn supports_update(&self) -> bool {
        false
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: crate::t_static!("cli-agent-plugin-oh-my-pi-install-title"),
    subtitle: crate::t_static!("cli-agent-plugin-oh-my-pi-install-subtitle"),
    steps: vec![
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-oh-my-pi-install-step-brew"),
            command: "brew install oh-my-pi/tap/oh-my-pi",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-oh-my-pi-install-step-cargo"),
            command: "cargo install oh-my-pi",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: vec![crate::t_static!(
        "cli-agent-plugin-oh-my-pi-restart-note"
    )],
});

static EMPTY_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "",
    subtitle: "",
    steps: vec![],
    post_install_notes: vec![],
});
