use std::sync::mpsc;

/// Eventos enviados do planning thread → TUI.
#[derive(Debug)]
pub enum LogEvent {
    Line(String),
    GateNeeded { content: String },
    Done,
    Failed(String),
}

/// Decisão do usuário no gate nativo do Ratatui → planning thread.
#[derive(Debug)]
pub enum GateDecision {
    Continue,
    Abort,
}

pub type LogTx = mpsc::SyncSender<LogEvent>;
pub type LogRx = mpsc::Receiver<LogEvent>;
pub type GateTx = mpsc::SyncSender<GateDecision>;
pub type GateRx = mpsc::Receiver<GateDecision>;
