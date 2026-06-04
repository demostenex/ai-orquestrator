//! Adapters de CLI de agente: a "receita" verificada para dirigir cada CLI
//! externa (binário + subcomando + flags + env) e limpar sua saída.
//!
//! Cada agente que passamos a usar ganha um adapter aqui. O executor genérico
//! (`run_cli`) resolve o adapter pelo binário e aplica `env()` antes de rodar e
//! `clean_output()` depois — assim o parser do agente sempre recebe stdout
//! limpo, sem o chamador precisar saber das flags.
//!
//! Extensão futura: backends via API (Anthropic/OpenAI/Gemini HTTP) podem
//! implementar o mesmo trait e, diferente das CLIs, reportar uso de tokens
//! (ver [`TokenUsage`]). Por isso o trait é pensado em torno de "backend de
//! agente", não apenas "CLI".

/// Uso de tokens reportado por um backend. Hoje só relevante para futuros
/// backends de API (CLIs não expõem isso); mantido como gancho de extensão.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

/// Como dirigir uma CLI de agente externa de forma não-interativa.
pub trait CliAdapter: Send + Sync {
    /// Identificador curto e estável (ex.: `"codex"`).
    fn id(&self) -> &'static str;
    /// Nome amigável exibido no picker (ex.: `"OpenAI Codex"`).
    fn display_name(&self) -> &'static str;
    /// Binário verificado no PATH para checar disponibilidade.
    fn binary(&self) -> &'static str;
    /// Comando shell completo rodado via `sh -c` (binário + subcomando + flags).
    fn command(&self) -> &'static str;
    /// Variáveis de ambiente extras necessárias ao modo não-interativo.
    fn env(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    /// Normaliza o stdout antes de devolver ao parser do agente.
    fn clean_output(&self, stdout: &str) -> String {
        stdout.trim().to_string()
    }
    /// Disponível no PATH?
    fn is_available(&self) -> bool {
        super::cli_runner::is_cli_available(self.binary())
    }
}

// ── Adapters built-in (receitas verificadas) ──────────────────────────────────

/// Google Gemini CLI. Saída limpa no stdout; precisa confiar no workspace.
pub struct Gemini;
impl CliAdapter for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }
    fn display_name(&self) -> &'static str {
        "Google Gemini"
    }
    fn binary(&self) -> &'static str {
        "gemini"
    }
    fn command(&self) -> &'static str {
        "gemini"
    }
    fn env(&self) -> &'static [(&'static str, &'static str)] {
        &[("GEMINI_CLI_TRUST_WORKSPACE", "true")]
    }
}

/// Anthropic Claude CLI em modo print (`-p`): não-interativo, stdout limpo.
pub struct Claude;
impl CliAdapter for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn display_name(&self) -> &'static str {
        "Anthropic Claude"
    }
    fn binary(&self) -> &'static str {
        "claude"
    }
    fn command(&self) -> &'static str {
        "claude -p"
    }
}

/// OpenAI Codex via `exec` (não-interativo). `--skip-git-repo-check` evita a
/// recusa em diretórios sem git. Banner vai para stderr; stdout fica limpo.
pub struct Codex;
impl CliAdapter for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn display_name(&self) -> &'static str {
        "OpenAI Codex"
    }
    fn binary(&self) -> &'static str {
        "codex"
    }
    fn command(&self) -> &'static str {
        "codex exec --skip-git-repo-check"
    }
}

// ── Registro ──────────────────────────────────────────────────────────────────

/// Todos os adapters built-in, na ordem de exibição.
pub fn builtin_adapters() -> Vec<Box<dyn CliAdapter>> {
    vec![Box::new(Gemini), Box::new(Claude), Box::new(Codex)]
}

/// Apenas os adapters cujo binário está disponível no PATH.
pub fn available_adapters() -> Vec<Box<dyn CliAdapter>> {
    builtin_adapters()
        .into_iter()
        .filter(|a| a.is_available())
        .collect()
}

/// Resolve um adapter a partir de um comando, casando pelo primeiro token
/// contra o `binary()` de cada adapter. Usado por `run_cli` para aplicar
/// `env()`/`clean_output()` mesmo quando o usuário digita a receita à mão.
pub fn adapter_for_command(cmd: &str) -> Option<Box<dyn CliAdapter>> {
    let bin = cmd.split_whitespace().next()?;
    builtin_adapters().into_iter().find(|a| a.binary() == bin)
}

/// Mapeia um input do usuário para a receita correta. Quando o input é um único
/// token que casa o `id` ou o `binary` de um adapter (ex.: `"codex"`), devolve
/// o `command()` completo (`"codex exec --skip-git-repo-check"`). Caso contrário
/// (comando já com flags, ou CLI desconhecida) devolve o input intacto.
pub fn resolve_command(input: &str) -> String {
    let trimmed = input.trim();
    if !trimmed.contains(char::is_whitespace) {
        if let Some(a) = builtin_adapters()
            .into_iter()
            .find(|a| a.id() == trimmed || a.binary() == trimmed)
        {
            return a.command().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_command_expands_known_ids() {
        assert_eq!(resolve_command("codex"), "codex exec --skip-git-repo-check");
        assert_eq!(resolve_command("claude"), "claude -p");
        assert_eq!(resolve_command("gemini"), "gemini");
        assert_eq!(resolve_command("  codex  "), "codex exec --skip-git-repo-check");
    }

    #[test]
    fn resolve_command_keeps_custom_and_flagged_commands() {
        // Comando já com flags não é clobberado.
        assert_eq!(resolve_command("codex exec --foo"), "codex exec --foo");
        // CLI desconhecida passa intacta.
        assert_eq!(resolve_command("minha-cli"), "minha-cli");
        assert_eq!(resolve_command("llm -m gpt-4o"), "llm -m gpt-4o");
    }

    #[test]
    fn adapter_for_command_matches_by_binary() {
        assert_eq!(
            adapter_for_command("codex exec --skip-git-repo-check")
                .map(|a| a.id()),
            Some("codex")
        );
        assert_eq!(adapter_for_command("gemini").map(|a| a.id()), Some("gemini"));
        assert!(adapter_for_command("desconhecida").is_none());
    }

    #[test]
    fn gemini_carries_trust_env() {
        let g = Gemini;
        assert_eq!(g.env(), &[("GEMINI_CLI_TRUST_WORKSPACE", "true")]);
        assert!(Claude.env().is_empty());
    }

    #[test]
    fn clean_output_trims_by_default() {
        assert_eq!(Codex.clean_output("  {\"ok\":true}\n\n"), "{\"ok\":true}");
    }
}
