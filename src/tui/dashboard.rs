use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::path::PathBuf;

use crate::core::db::{Db, EventRow};
use crate::schemas::Task;

// ── State de dados ────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DashboardState {
    pub plan_id: String,
    pub plan_title: String,
    pub tasks: Vec<Task>,
    pub recent_events: Vec<EventRow>,
}

impl DashboardState {
    pub async fn load(orchestrator_dir: PathBuf, plan_id: String) -> Result<Self> {
        let db = Db::open_readonly(&orchestrator_dir).await?;
        let plan_title = db.get_plan_title(&plan_id).await?;
        let tasks = db.get_plan_tasks(&plan_id).await?;
        let recent_events =
            Db::list_recent_events_for_plan(orchestrator_dir.clone(), plan_id.clone(), 30).await?;
        Ok(Self { plan_id, plan_title, tasks, recent_events })
    }
}

// ── Estado de UI (navegação + memória) ───────────────────────────────────────

pub struct DashboardUiState {
    pub list_state: ListState,
    pub memory_content: Option<String>,
}

impl DashboardUiState {
    pub fn new(event_count: usize) -> Self {
        let mut list_state = ListState::default();
        if event_count > 0 {
            list_state.select(Some(0));
        }
        Self { list_state, memory_content: None }
    }

    pub fn reset(&mut self, event_count: usize) {
        self.list_state = ListState::default();
        self.memory_content = None;
        if event_count > 0 {
            self.list_state.select(Some(0));
        }
    }
}

// ── Comandos emitidos pelo handler de teclas ──────────────────────────────────

pub enum DashboardCmd {
    None,
    Reload,
    LoadMemory(String), // run_id do evento selecionado
    Back,
}

pub fn handle_key(key: KeyEvent, ui: &mut DashboardUiState, ds: &DashboardState) -> DashboardCmd {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => DashboardCmd::Back,
        KeyCode::Char('r') => DashboardCmd::Reload,
        KeyCode::Up => {
            let i = ui.list_state.selected().unwrap_or(0);
            if i > 0 {
                ui.list_state.select(Some(i - 1));
            }
            DashboardCmd::None
        }
        KeyCode::Down => {
            let len = ds.recent_events.len();
            let i = ui.list_state.selected().unwrap_or(0);
            if i + 1 < len {
                ui.list_state.select(Some(i + 1));
            }
            DashboardCmd::None
        }
        KeyCode::Enter => {
            if let Some(selected) = ui.list_state.selected() {
                let len = ds.recent_events.len();
                if let Some(ev) = ds.recent_events.get(len.saturating_sub(1 + selected)) {
                    return DashboardCmd::LoadMemory(ev.run_id.clone());
                }
            }
            DashboardCmd::None
        }
        _ => DashboardCmd::None,
    }
}

// ── Render (puro — sem loop, sem raw mode) ────────────────────────────────────

/// Renderiza o corpo do dashboard na área fornecida.
/// Inclui mini-header de progresso + 3 colunas.
pub fn render_body(
    f: &mut Frame,
    ds: &DashboardState,
    ui: &mut DashboardUiState,
    area: Rect,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Mini-header: plano + progresso
            Constraint::Min(5),    // 3 colunas
        ])
        .split(area);

    // Mini-header
    let progress = compute_progress(&ds.tasks);
    let header_text = format!("{}  │  {}", ds.plan_title, progress);
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    format!(" Plano: {} ", &ds.plan_id[..8.min(ds.plan_id.len())]),
                    Style::default().fg(Color::Cyan),
                )),
        );
    f.render_widget(header, chunks[0]);

    // 3 colunas
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(40),
            Constraint::Percentage(25),
        ])
        .split(chunks[1]);

    render_tasks(f, &ds.tasks, body[0]);
    render_audit_feed(f, ds, ui, body[1]);
    render_memory_panel(f, ds, ui, body[2]);
}

fn render_tasks(f: &mut Frame, tasks: &[Task], area: Rect) {
    let items: Vec<ListItem> = tasks
        .iter()
        .map(|task| {
            let (symbol, color) = match task.status.as_str() {
                "completed" => ("✅", Color::Green),
                "in_progress" => ("⏳", Color::Yellow),
                "blocked" => ("[!]", Color::Red),
                _ => ("[ ]", Color::Gray),
            };
            ListItem::new(format!("{} {}", symbol, task.description))
                .style(Style::default().fg(color))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Tarefas do Plano"));
    f.render_widget(list, area);
}

fn render_audit_feed(f: &mut Frame, ds: &DashboardState, ui: &mut DashboardUiState, area: Rect) {
    let items: Vec<ListItem> = ds
        .recent_events
        .iter()
        .rev()
        .map(|ev| {
            let color = match ev.event_type.as_str() {
                "security_blocked" | "audit_rejected" => Color::Red,
                s if s.contains("response") || s.contains("prompt") => Color::Cyan,
                _ => Color::Gray,
            };
            let text = format!(
                "[{}] {} → {}",
                &ev.timestamp[..16],
                ev.from_agent.as_deref().unwrap_or("-"),
                ev.content_summary.as_deref().unwrap_or(""),
            );
            ListItem::new(text).style(Style::default().fg(color))
        })
        .collect();

    let feed = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Audit Feed (↑↓ navega, Enter carrega)"),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Rgb(40, 40, 60)),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(feed, area, &mut ui.list_state);
}

fn render_memory_panel(f: &mut Frame, ds: &DashboardState, ui: &mut DashboardUiState, area: Rect) {
    let len = ds.recent_events.len();
    let selected_event = ui
        .list_state
        .selected()
        .and_then(|i| ds.recent_events.get(len.saturating_sub(1 + i)));

    let text: String = match (selected_event, ui.memory_content.as_deref()) {
        (Some(ev), Some(content)) => format!(
            "── Evento #{} ──\nTipo : {}\nDe   : {} → {}\nTs   : {}\n\n{}\n────────────────\n\n{}",
            ev.sequence,
            ev.event_type,
            ev.from_agent.as_deref().unwrap_or("-"),
            ev.to_agent.as_deref().unwrap_or("-"),
            &ev.timestamp[..19],
            ev.content_summary.as_deref().unwrap_or("(sem resumo)"),
            content,
        ),
        (Some(ev), None) => format!(
            "── Evento #{} ──\nTipo : {}\nDe   : {} → {}\nTs   : {}\n\n{}\n────────────────\n\nEnter → carregar handoff",
            ev.sequence,
            ev.event_type,
            ev.from_agent.as_deref().unwrap_or("-"),
            ev.to_agent.as_deref().unwrap_or("-"),
            &ev.timestamp[..19],
            ev.content_summary.as_deref().unwrap_or("(sem resumo)"),
        ),
        (None, _) => "Selecione um evento\ncom ↑↓ para ver\no contexto.\n\nEnter → carregar\nhandoff do ai-memory".to_string(),
    };

    let panel = Paragraph::new(text.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Memória (AI-Memory)"),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(panel, area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn compute_progress(tasks: &[Task]) -> String {
    let total = tasks.len();
    if total == 0 {
        return "Sem tarefas".to_string();
    }
    let completed = tasks.iter().filter(|t| t.status == "completed").count();
    let pct = (completed * 100) / total;
    let filled = (completed * 20) / total;
    let bar: String = (0..20).map(|i| if i < filled { '█' } else { '░' }).collect();
    format!("[{}] {}% ({}/{})", bar, pct, completed, total)
}

/// Invoca o ai-memory CLI para ler o handoff associado a um run_id.
pub fn load_memory_for_run(run_id: &str) -> String {
    let direct_path = format!("handoffs/run_{}.md", run_id);

    if let Ok(output) = std::process::Command::new("ai-memory")
        .args(["read-page", "--path", &direct_path])
        .output()
    {
        if output.status.success() {
            let body = String::from_utf8_lossy(&output.stdout).to_string();
            if !body.trim().is_empty() {
                return format!("📄 Handoff do run {}\n\n{}", run_id, body);
            }
        }
    }

    if let Ok(output) = std::process::Command::new("ai-memory")
        .args(["read-page", run_id])
        .output()
    {
        if output.status.success() {
            let body = String::from_utf8_lossy(&output.stdout).to_string();
            if !body.trim().is_empty() {
                return format!("🔍 Busca por run_id {}\n\n{}", run_id, body);
            }
        }
    }

    format!(
        "Nenhuma página encontrada\npara o run_id:\n{}\n\n(handoffs são registrados\nvia write-page em cada ciclo)",
        run_id
    )
}
