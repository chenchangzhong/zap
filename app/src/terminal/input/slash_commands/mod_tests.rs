//! Unit tests for the slash-command execution module.

use super::slash_command_is_submitted_as_prompt;
use crate::search::slash_command_menu::static_commands::commands;

/// The centralized classifier must mark only the prompt-submitting commands (/init, /plan) as
/// "submitted as a prompt". Every other slash command emits an immediate action and must be
/// treated as "run now" by the prompt-queue gate and the shared-session viewer path.
///
/// 本地命令表与上游不同:无 `ORCHESTRATE` 与 `CONTINUE_LOCALLY`,断言列表按本地命令表裁剪;
/// 且本地 `/compact` 走独立 arm(不是 prompt-prefix 臂),因此也不在 prompt 名单内。
#[test]
fn slash_command_is_submitted_as_prompt_only_for_prompt_commands() {
    // Prompt-submitting commands reiterate their text into the conversation.
    assert!(slash_command_is_submitted_as_prompt(&commands::INIT));
    assert!(slash_command_is_submitted_as_prompt(&commands::PLAN));

    // Action-emitting commands run immediately and are never queued / forwarded as prompts.
    assert!(!slash_command_is_submitted_as_prompt(&commands::COMPACT));
    assert!(!slash_command_is_submitted_as_prompt(
        &commands::COMPACT_AND
    ));
    assert!(!slash_command_is_submitted_as_prompt(&commands::FORK));
    assert!(!slash_command_is_submitted_as_prompt(
        &commands::FORK_AND_COMPACT
    ));
    assert!(!slash_command_is_submitted_as_prompt(&commands::FORK_FROM));
    assert!(!slash_command_is_submitted_as_prompt(&commands::MODEL));
    assert!(!slash_command_is_submitted_as_prompt(&commands::REWIND));
    assert!(!slash_command_is_submitted_as_prompt(
        &commands::CONVERSATIONS
    ));
    assert!(!slash_command_is_submitted_as_prompt(&commands::QUEUE));
}
