use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
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
    let mut audit_list_state = ListState::default();
    let mut memory_content: Option<String> = None;

    // Seleciona o primeiro evento por padrão (se houver)
    if !state.recent_events.is_empty() {
        audit_list_state.select(Some(0));
    }

    loop {
        terminal.draw(|f| ui(f, &state, &mut audit_list_state, memory_content.as_deref()))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('r') => {
                            let new_state = {
                                let rt = tokio::runtime::Runtime::new()?;
                                rt.block_on(DashboardState::load(
                                    orchestrator_dir.clone(),
                                    plan_id.clone(),
                                ))?
                            };
                            state = new_state;
                            // Reset UI state após reload
                            audit_list_state = ListState::default();
                            memory_content = None;
                            if !state.recent_events.is_empty() {
                                audit_list_state.select(Some(0));
                            }
                        }
                        KeyCode::Up => {
                            if let Some(selected) = audit_list_state.selected() {
                                if selected > 0 {
                                    audit_list_state.select(Some(selected - 1));
                                }
                            }
                        }
                        KeyCode::Down => {
                            let len = state.recent_events.len();
                            if let Some(selected) = audit_list_state.selected() {
                                if selected + 1 < len {
                                    audit_list_state.select(Some(selected + 1));
                                }
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(selected) = audit_list_state.selected() {
                                // Lista exibida é events.iter().rev(), então índice i → events[len-1-i]
                                let len = state.recent_events.len();
                                if let Some(ev) = state.recent_events.get(len.saturating_sub(1 + selected)) {
                                    memory_content = Some(load_memory_for_run(&ev.run_id));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn ui(
    f: &mut Frame,
    state: &DashboardState,
    audit_list_state: &mut ListState,
    memory_content: Option<&str>,
) {
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

    // === BODY: Horizontal 3 colunas ===
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35), // Tarefas
            Constraint::Percentage(40), // Audit Feed (stateful)
            Constraint::Percentage(25), // Memória (ai-memory)
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

    // Coluna 1: Audit Feed stateful (navegação com ↑↓ + Enter para carregar memória)
    let event_items: Vec<ListItem> = state
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
                ev.content_summary.as_deref().unwrap_or("")
            );
            ListItem::new(text).style(Style::default().fg(color))
        })
        .collect();

    let audit_feed = List::new(event_items)
        .block(Block::default().borders(Borders::ALL).title("Audit Feed (↑↓ navega, Enter carrega)"))
        .highlight_style(Style::default().fg(Color::Yellow).bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(audit_feed, body_chunks[1], audit_list_state);

    // Coluna 2: Memória — contexto do evento selecionado + handoff carregado
    let len = state.recent_events.len();
    let selected_event = audit_list_state
        .selected()
        .and_then(|i| state.recent_events.get(len.saturating_sub(1 + i)));

    let memory_text: String = match (selected_event, memory_content) {
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
            "── Evento #{} ──\nTipo : {}\nDe   : {} → {}\nTs   : {}\n\n{}\n────────────────\n\nEnter → carregar handoff do ai-memory",
            ev.sequence,
            ev.event_type,
            ev.from_agent.as_deref().unwrap_or("-"),
            ev.to_agent.as_deref().unwrap_or("-"),
            &ev.timestamp[..19],
            ev.content_summary.as_deref().unwrap_or("(sem resumo)"),
        ),
        (None, _) => "Selecione um evento com ↑↓\nEnter → carregar handoff do ai-memory\n\n(Fase 4.4 ativa)".to_string(),
    };

    let memory_panel = Paragraph::new(memory_text.as_str())
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title("Memória (AI-Memory)"))
        .wrap(Wrap { trim: true });
    f.render_widget(memory_panel, body_chunks[2]);

    // === FOOTER ===
    let footer = Paragraph::new("q: Sair  |  ↑↓: Navegar  |  Enter: Carregar Handoff  |  r: Recarregar  |  Fase 4.4")
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

/// Invoca o ai-memory CLI para ler a página de handoff associada a um run_id.
/// Usa `read-page --path` (com fallback para busca por run_id).
/// Retorna o conteúdo ou mensagem de erro amigável.
fn load_memory_for_run(run_id: &str) -> String {
    // Tentativa 1: caminho canônico de handoff por run
    let direct_path = format!("handoffs/run_{}.md", run_id);

    if let Ok(output) = std::process::Command::new("ai-memory")
        .args(["read-page", "--path", &direct_path])
        .output()
    {
        if output.status.success() {
            let body = String::from_utf8_lossy(&output.stdout).to_string();
            if !body.trim().is_empty() {
                return format!("📄 Handoff vinculado ao run {}\n\n{}", run_id, body);
            }
        }
    }

    // Tentativa 2: busca textual pelo run_id (FTS5)
    if let Ok(output) = std::process::Command::new("ai-memory")
        .args(["read-page", run_id])
        .output()
    {
        if output.status.success() {
            let body = String::from_utf8_lossy(&output.stdout).to_string();
            if !body.trim().is_empty() {
                return format!("🔍 Busca por run_id {} (melhor match)\n\n{}", run_id, body);
            }
        }
    }

    format!(
        "Nenhuma página de handoff encontrada no ai-memory para o run_id:\n{}\n\n\
         (Fase 4.3b: handoffs são sincronizados via write-page em etapas anteriores do ciclo)",
        run_id
    )
}