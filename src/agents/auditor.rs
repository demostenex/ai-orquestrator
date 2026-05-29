use anyhow::{Context, Result};

use crate::agents::AgentCall;
use crate::core::strip_json_fences;
use crate::providers::base::{ChatMessage, Provider};
use crate::schemas::{AuditResponse, Handoff};

const SYSTEM_PROMPT: &str = r#"You are an AI Auditor agent. Your ONLY output must be valid JSON matching exactly:
{
  \"approved\": <bool>,
  \"score\": <integer 0-100>,
  \"problems\": [\"<string>\"],
  \"required_changes\": [\"<string>\"],
  \"blocked_reason\": <string or null>
}
No markdown, no explanation, no code blocks. Raw JSON only."#;

pub async fn execute(
    provider: &dyn Provider,
    step_id: &str,
    plan: &str,
    memory: &str,
    handoff: Option<&Handoff>,
    diff: &str,
    files_modified: &[String],
    apply_check_result: &str,
) -> Result<AgentCall<AuditResponse>> {
    let prompt = build_user_prompt(
        step_id,
        plan,
        memory,
        handoff,
        diff,
        files_modified,
        apply_check_result,
    );

    let provider_response = provider
        .complete(
            SYSTEM_PROMPT,
            vec![ChatMessage {
                role: "user".to_string(),
                content: prompt.clone(),
            }],
        )
        .await?;

    let clean_json = strip_json_fences(&provider_response.text);
    let parsed: AuditResponse = serde_json::from_str(clean_json)
        .with_context(|| "failed to parse IA Auditor JSON response")?;
    parsed.validate()?;

    Ok(AgentCall {
        parsed,
        prompt,
        provider_response,
    })
}

pub fn system_prompt() -> &'static str {
    SYSTEM_PROMPT
}

pub fn build_user_prompt(
    step_id: &str,
    plan: &str,
    memory: &str,
    handoff: Option<&Handoff>,
    diff: &str,
    files_modified: &[String],
    apply_check_result: &str,
) -> String {
    let handoff_text = handoff
        .map(|value| serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "No handoff available.".to_string());

    format!(
        "Step atual: {step_id}\n\n[PLAN]\n{plan}\n\n[MEMORY]\n{memory}\n\n[HANDOFF]\n{handoff_text}\n\n[FILES_MODIFIED]\n{}\n\n[PATCH]\n{diff}\n\n[GIT_APPLY_CHECK]\n{apply_check_result}\n\nAvalie segurança, escopo, aplicabilidade e qualidade geral. Responda somente no JSON solicitado.",
        if files_modified.is_empty() {
            "(none)".to_string()
        } else {
            files_modified.join(", ")
        }
    )
}
