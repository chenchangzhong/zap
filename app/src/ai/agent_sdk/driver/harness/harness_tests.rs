use super::{resolve_cli_command, validate_cli_installed};
use crate::ai::agent_sdk::driver::AgentDriverError;

#[test]
fn test_resolve_cli_command_finds_binary_on_process_path() {
    // `env` 一定在进程 PATH 中,验证快速路径解析。
    assert!(resolve_cli_command("env").is_some());
}

#[test]
fn test_resolve_cli_command_returns_none_for_missing_binary() {
    // 随机后缀确保不命中本机任何兜底安装目录(如 /opt/homebrew/bin)。
    let probe = format!("zap-cli-probe-{:x}", std::process::id());
    assert!(resolve_cli_command(&probe).is_none());
}

#[test]
fn test_validate_cli_installed_accepts_known_binary() {
    assert!(validate_cli_installed("env", None).is_ok());
}

#[test]
fn test_validate_cli_installed_reports_missing_binary() {
    let probe = format!("zap-cli-probe-{:x}", std::process::id());
    let err = validate_cli_installed(&probe, Some("https://example.com/install"))
        .expect_err("缺失的 CLI 必须返回 HarnessSetupFailed");
    match err {
        AgentDriverError::HarnessSetupFailed { harness, reason } => {
            assert_eq!(harness, probe);
            assert!(reason.contains(&probe));
            assert!(reason.contains("https://example.com/install"));
        }
        other => panic!("期望 HarnessSetupFailed,实际 {other:?}"),
    }
}
