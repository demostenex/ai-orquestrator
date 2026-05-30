use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::io;
use std::path::PathBuf;

use crate::core::db::{Db, EventRow};
use crate::schemas::Task;

/// Estado do Dashboard - armazena dados carregados do SQLite para evitar queries por frame.
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

        // Fase 4.3a: usa a query JOIN por plan_id → run_id (sem alterar schema)
        // Retorna [] se o plano ainda não tiver run_id associado (comportamento correto)
        let recent_events = Db::list_recent_events_for_plan(
            orchestrator_dir.clone(),
            plan_id.clone(),
            30,
        )
        .await?;

        Ok(Self {
            plan_id,
            plan_title,
            tasks,
            recent_events,
        })
    }
}

/// Ponto de entrada do Dashboard em Ratatui (Fase 4 - Read Only).
pub fn run_dashboard(orchestrator_dir: PathBuf, plan_id: String) -> Result<()> {
    // Carrega estado de forma assíncrona antes de entrar no modo raw
    let rt = tokio::runtime::Runtime::new()?;
    let state = rt.block_on(DashboardState::load(orchestrator_dir.clone(), plan_id.clone()))?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, state, orchestrator_dir, plan_id);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut state: DashboardState,
    orchestrator_dir: PathBuf,
    plan_id: String,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, &state))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('r') => {
                            // Reload: recria runtime local (barato para uso esporádico)
                            let new_state = {
                                let rt = tokio::runtime::Runtime::new()?;
                                rt.block_on(DashboardState::load(
                                    orchestrator_dir.clone(),
                                    plan_id.clone(),
                                ))?
                            };
                            state = new_state;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn ui(f: &mut Frame, state: &DashboardState) {
    let size = f.area();

    // Layout principal: Vertical (Header | Body | Footer)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Header (2 linhas + bordas)
            Constraint::Min(5),    // Body
            Constraint::Length(3), // Footer
        ])
        .split(size);

    // === HEADER com Barra de Progresso (Fase 4.3a) ===
    let progress = compute_progress(&state.tasks);
    let header_text = format!("AI Orchestrator — {}\n{}", state.plan_title, progress);
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::ALL).title(format!("Plano: {}", state.plan_id)));
    f.render_widget(header, chunks[0]);

    // === BODY: Horizontal 3 colunas (prep para Fase 4.3b) ===
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35), // Tarefas
            Constraint::Percentage(40), // Audit Feed
            Constraint::Percentage(25), // Memory (placeholder)
        ])
        .split(chunks[1]);

    // Coluna 0: Lista de Tarefas (real + cores por status)
    let task_items: Vec<ListItem> = state
        .tasks
        .iter()
        .map(|task| {
            let (symbol, color) = match task.status.as_str() {
                "completed" => ("✅", Color::Green),
                "in_progress" => ("⏳", Color::Yellow),
                "blocked" => ("[!]", Color::Red),
                _ => ("[ ]", Color::Gray),
            };
            let text = format!("{} {}", symbol, task.description);
            ListItem::new(text).style(Style::default().fg(color))
        })
        .collect();

    let tasks_list = List::new(task_items)
        .block(Block::default().borders(Borders::ALL).title("Tarefas do Plano"));
    f.render_widget(tasks_list, body_chunks[0]);

    // Coluna 1: Feed de Auditoria (dados reais via JOIN plan→run, invertido para fluxo cronológico)
    let event_items: Vec<ListItem> = state
        .recent_events
        .iter()
        .take(8) // 8 mais recentes (query ORDER BY timestamp DESC)
        .rev()    // inverte para mais antigo (do recorte) no topo → fluxo de baixo para cima
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
                ev.content_summary.as_deref().unwrap_or("")
            );
            ListItem::new(text).style(Style::default().fg(color))
        })
        .collect();

    let audit_feed = List::new(event_items)
        .block(Block::default().borders(Borders::ALL).title("Audit Feed (eventos do plano)"));
    f.render_widget(audit_feed, body_chunks[1]);

    // Coluna 2: Placeholder Memory (Fase 4.3b trará navegação + ai-memory)
    let memory_panel = Paragraph::new("Aguardando Seleção...\n\n(Fase 4.3b: Navegação + AI-Memory)")
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL).title("Memória (AI-Memory)"));
    f.render_widget(memory_panel, body_chunks[2]);

    // === FOOTER ===
    let footer = Paragraph::new("q: Sair  |  r: Recarregar  |  Fase 4.3a - Progress + Reload + Real Events por Plano")
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

/// Calcula barra de progresso Unicode usando █ (cheio) e ░ (vazio).
fn compute_progress(tasks: &[Task]) -> String {
    let total = tasks.len();
    if total == 0 {
        return "Sem tarefas".to_string();
    }
    let completed = tasks.iter().filter(|t| t.status == "completed").count();
    let pct = (completed * 100) / total;
    let width = 20usize;
    let filled = (completed * width) / total;
    let bar: String = (0..width)
        .map(|i| if i < filled { '█' } else { '░' })
        .collect();
    format!("Progresso: [{}] {}% ({}/{})", bar, pct, completed, total)
}