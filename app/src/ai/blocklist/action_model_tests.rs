use std::{collections::HashMap, sync::Arc};

use super::*;
use crate::ai::agent::{task::TaskId, AIAgentActionResultType};

fn make_action_result(id: &str) -> Arc<AIAgentActionResult> {
    Arc::new(AIAgentActionResult {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: AIAgentActionResultType::InitProject,
    })
}

fn count_startable_actions_for_pass(phases: &[(RunningActionPhase, bool)]) -> usize {
    let mut current_phase = None;
    let mut count = 0;

    for (phase, can_autoexecute) in phases {
        if let Some(current_phase) = current_phase {
            if !can_start_action_with_current_phase(current_phase, *phase, *can_autoexecute) {
                break;
            }
        }

        count += 1;
        current_phase = Some(*phase);

        if matches!(*phase, RunningActionPhase::Serial) {
            break;
        }
    }

    count
}

#[test]
fn parallel_phase_only_admits_matching_autoexecutable_actions() {
    let phase =
        RunningActionPhase::Parallel(execute::ParallelExecutionPolicy::ReadOnlyLocalContext);

    assert!(can_start_action_with_current_phase(phase, phase, true));
    assert!(!can_start_action_with_current_phase(phase, phase, false));
    assert!(!can_start_action_with_current_phase(
        phase,
        RunningActionPhase::Serial,
        true
    ));
    assert!(!can_start_action_with_current_phase(
        RunningActionPhase::Serial,
        phase,
        true
    ));
}

#[test]
fn phased_scheduling_stops_at_serial_barrier_and_resumes_afterward() {
    let read_only_phase =
        RunningActionPhase::Parallel(execute::ParallelExecutionPolicy::ReadOnlyLocalContext);
    let actions = vec![
        (read_only_phase, true),
        (read_only_phase, true),
        (RunningActionPhase::Serial, true),
        (read_only_phase, true),
        (read_only_phase, true),
    ];

    assert_eq!(count_startable_actions_for_pass(&actions), 2);
    assert_eq!(count_startable_actions_for_pass(&actions[2..]), 1);
    assert_eq!(count_startable_actions_for_pass(&actions[3..]), 2);
}

#[test]
fn finished_results_stay_in_original_action_order() {
    let action_order = HashMap::from([
        (AIAgentActionId::from("first".to_owned()), 0),
        (AIAgentActionId::from("second".to_owned()), 1),
        (AIAgentActionId::from("third".to_owned()), 2),
    ]);
    let mut finished_results = [
        make_action_result("third"),
        make_action_result("first"),
        make_action_result("second"),
    ];

    finished_results
        .sort_by_key(|result| action_order.get(&result.id).copied().unwrap_or(usize::MAX));

    assert_eq!(
        finished_results[0].id,
        AIAgentActionId::from("first".to_owned())
    );
    assert_eq!(
        finished_results[1].id,
        AIAgentActionId::from("second".to_owned())
    );
    assert_eq!(
        finished_results[2].id,
        AIAgentActionId::from("third".to_owned())
    );
}

/// 动作名必须走 i18n 且**不带调试引号**。
///
/// 之前的调用点写成 `format!("{...:?}")`,把文案包成了 `"Run command: ls"`,
/// 引号会原样显示到状态卡上;这个用例把「纯文本」这条契约锁住。
#[test]
fn localized_action_name_is_translated_and_unquoted() {
    crate::i18n::init(Some("zh-CN"));

    let run = localized_action_name(&AIAgentActionType::RequestCommandOutput {
        command: "ls -la".to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: false,
        uses_pager: None,
        rationale: None,
        citations: vec![],
    });
    assert!(run.contains("ls -la"), "命令内容必须保留,实际: {run}");
    assert!(!run.starts_with('"'), "不得带调试引号,实际: {run}");
    assert!(!run.contains("agent-action-name"), "键未解析: {run}");

    let read = localized_action_name(&AIAgentActionType::ReadFiles(
        crate::ai::agent::ReadFilesRequest { locations: vec![] },
    ));
    assert!(!read.contains("agent-action-name"), "键未解析: {read}");
    assert!(
        !read.eq_ignore_ascii_case("Read files"),
        "中文 locale 下不应回退英文,实际: {read}"
    );

    // 英文 locale 下同样是解析后的文本(而不是 key)。
    crate::i18n::init(Some("en"));
    let grep = localized_action_name(&AIAgentActionType::Grep {
        queries: vec!["foo".to_owned()],
        path: ".".to_owned(),
    });
    assert!(!grep.contains("agent-action-name"), "键未解析: {grep}");
}
