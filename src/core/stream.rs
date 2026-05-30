use std::sync::mpsc;

/// Eventos enviados do planning/dev thread → TUI.
#[derive(Debug)]
pub enum LogEvent {
    Line(String),
    /// gate_type: "planning" | "diff_review" | "apply" | "inter_task" | "error"
    GateNeeded { content: String, gate_type: String },
    Done,
    Failed(String),
}

/// Decisão do usuário no gate nativo do Ratatui → planning thread.
#[derive(Debug)]
pub enum GateDecision {
    Continue,
    Enrich(String), // notas humanas aplicadas antes do próximo turno
    Finalize,
    Abort,
}

pub type LogTx = mpsc::SyncSender<LogEvent>;
pub type LogRx = mpsc::Receiver<LogEvent>;
pub type GateTx = mpsc::SyncSender<GateDecision>;
pub type GateRx = mpsc::Receiver<GateDecision>;
