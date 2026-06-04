use std::sync::mpsc;

/// Origem estruturada de uma linha de log, quando o produtor conhece o autor
/// na fonte. Permite ao TUI rotear a linha ao pane correto sem depender de
/// heurística de string (que continua como fallback para stdout cru de CLIs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
    Architect,
    Reviewer,
    Dev,
    Auditor,
}

impl LineOrigin {
    /// Converte o nome de papel usado no planning ("architect", "reviewer", …).
    pub fn from_role(role: &str) -> Option<Self> {
        match role.trim().to_lowercase().as_str() {
            "architect" | "arquiteto" => Some(Self::Architect),
            "reviewer" | "revisor" => Some(Self::Reviewer),
            "dev" => Some(Self::Dev),
            "auditor" | "auditora" => Some(Self::Auditor),
            _ => None,
        }
    }
}

/// Eventos enviados do planning/dev thread → TUI.
#[derive(Debug)]
pub enum LogEvent {
    /// Linha de origem desconhecida (ex.: stdout cru de CLI) → heurística no TUI.
    Line(String),
    /// Linha cujo autor é conhecido na fonte → roteamento estruturado no TUI.
    AgentLine {
        text: String,
        origin: LineOrigin,
    },
    /// gate_type: "planning" | "diff_review" | "apply" | "inter_task" | "error"
    GateNeeded {
        content: String,
        gate_type: String,
    },
    PlanningReady {
        plan_id: String,
    },
    Done,
    Failed(String),
}

/// Decisão do usuário no gate nativo do Ratatui → planning thread.
#[derive(Debug)]
pub enum GateDecision {
    Continue,
    Review,
    Enrich(String), // notas humanas aplicadas antes do próximo turno
    Finalize,
    Abort,
}

pub type LogTx = mpsc::SyncSender<LogEvent>;
pub type LogRx = mpsc::Receiver<LogEvent>;
pub type GateTx = mpsc::SyncSender<GateDecision>;
pub type GateRx = mpsc::Receiver<GateDecision>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_role_maps_known_roles_case_insensitively() {
        assert_eq!(
            LineOrigin::from_role("architect"),
            Some(LineOrigin::Architect)
        );
        assert_eq!(
            LineOrigin::from_role("Arquiteto"),
            Some(LineOrigin::Architect)
        );
        assert_eq!(
            LineOrigin::from_role("  REVIEWER "),
            Some(LineOrigin::Reviewer)
        );
        assert_eq!(LineOrigin::from_role("dev"), Some(LineOrigin::Dev));
        assert_eq!(LineOrigin::from_role("Auditora"), Some(LineOrigin::Auditor));
    }

    #[test]
    fn from_role_rejects_unknown_role() {
        assert_eq!(LineOrigin::from_role("qa"), None);
        assert_eq!(LineOrigin::from_role(""), None);
    }
}
