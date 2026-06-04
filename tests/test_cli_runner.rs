use ai_orchestrator::core::cli_runner::run_cli;

#[test]
fn run_cli_trusts_gemini_workspace_for_child_processes() {
    let output = run_cli("printf '%s' \"$GEMINI_CLI_TRUST_WORKSPACE\"", "", None)
        .expect("run_cli should execute printf");

    assert_eq!(output.trim(), "true");
}
