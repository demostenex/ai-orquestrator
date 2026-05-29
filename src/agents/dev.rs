use anyhow::{Context, Result};

use crate::agents::AgentCall;
use crate::core::strip_json_fences;
use crate::providers::base::{ChatMessage, Provider};
use crate::schemas::{DevResponse, Handoff};

const SYSTEM_PROMPT: &str = r#"You are an AI Dev agent. Your ONLY output must be valid JSON matching exactly:
{
  \"step_id\": \"<string>\",
  \"summary\": \"<string>\",
  \"files_touched\": [\"<string>\"],
  \"diff\": \"<unified diff string>\",
  \"tests_suggested\": [\"<string>\"],
  \"risks\": [\"<string>\"]
}
No markdown, no explanation, no code blocks. Raw JSON only."#;

pub async fn execute(
    provider: &dyn Provider,
    step_id: &str,
    plan: &str,
    memory: &str,
    last_handoff: Option<&Handoff>,
    workspace_snapshot: &str,
) -> Result<AgentCall<DevResponse>> {
    let prompt = build_user_prompt(step_id, plan, memory, last_handoff, workspace_snapshot);
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
    let parsed: DevResponse = serde_json::from_str(clean_json)
        .with_context(|| "failed to parse IA Dev JSON response")?;
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
    last_handoff: Option<&Handoff>,
    workspace_snapshot: &str,
) -> String {
    let handoff_text = last_handoff
        .map(|handoff| serde_json::to_string_pretty(handoff).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "Nenhum handoff anterior.".to_string());

    format!(
        "Step atual: {step_id}\n\nContexto obrigatório:\n[PLAN]\n{plan}\n\n[MEMORY]\n{memory}\n\n[WORKSPACE]\n{workspace_snapshot}\n\n[LAST_HANDOFF]\n{handoff_text}\n\nRegras: gere apenas diff unified, respeite o escopo do plano, não inclua segredos e não explique nada fora do JSON solicitado."
    )
}

/// Lê os arquivos de texto do workspace e retorna um snapshot para o prompt do Dev.
/// Ignora .git, .ai-orchestrator, binários e arquivos grandes (>64KB).
pub fn build_workspace_snapshot(workspace: &std::path::Path) -> String {
    use std::fs;

    let ignore = [".git", ".ai-orchestrator", "target", "node_modules", ".venv", "__pycache__"];
    let max_bytes = 64 * 1024;
    let mut out = String::new();

    let Ok(entries) = fs::read_dir(workspace) else { return "(não foi possível ler o workspace)".to_string() };

    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();

    fn collect(base: &std::path::Path, rel: &std::path::Path, ignore: &[&str], max_bytes: usize, out: &mut String) {
        let full = base.join(rel);
        let Ok(entries) = std::fs::read_dir(&full) else { return };
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if ignore.iter().any(|i| name == *i) { continue; }
            let rel_path = path.strip_prefix(base).unwrap_or(&path);
            if path.is_dir() {
                collect(base, rel_path, ignore, max_bytes, out);
            } else if path.is_file() {
                let Ok(meta) = std::fs::metadata(&path) else { continue };
                if meta.len() > max_bytes as u64 { continue; }
                let Ok(content) = std::fs::read_to_string(&path) else { continue };
                out.push_str(&format!("### {}\n```\n{}\n```\n\n", rel_path.display(), content.trim_end()));
            }
        }
    }

    for path in &paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if ignore.iter().any(|i| name == *i) { continue; }
        let rel = path.strip_prefix(workspace).unwrap_or(path);
        if path.is_dir() {
            collect(workspace, rel, &ignore, max_bytes, &mut out);
        } else if path.is_file() {
            let Ok(meta) = std::fs::metadata(path) else { continue };
            if meta.len() > max_bytes as u64 { continue; }
            let Ok(content) = std::fs::read_to_string(path) else { continue };
            out.push_str(&format!("### {}\n```\n{}\n```\n\n", rel.display(), content.trim_end()));
        }
    }

    if out.is_empty() {
        "(workspace vazio — nenhum arquivo de texto encontrado)".to_string()
    } else {
        out
    }
}
