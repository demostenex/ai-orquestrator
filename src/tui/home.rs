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

use crate::core::db::ProjectSummary;

// ── Menu ──────────────────────────────────────────────────────────────────────

const MENU_ITEMS: &[(&str, &str)] = &[
    ("1. Novo Plano", "Cria um novo plano com o Arquiteto"),
    ("2. Continuar Planejamento", "Retoma uma sessão de planejamento em andamento"),
    ("3. Executar Modo Dev", "Executa as tarefas de um plano aprovado"),
    ("4. Abrir Dashboard", "Visualiza progresso e auditoria em tempo real"),
    ("5. Sair", "Encerra o AI Orchestrator"),
];

// ── Actions ───────────────────────────────────────────────────────────────────

pub enum HomeAction {
    NewPlan,
    ContinuePlanning,
    ExecuteDev,
    OpenDashboard,
    Quit,
}

// ── State ─────────────────────────────────────────────────────────────────────

struct HomeState {
    menu_state: ListState,
    summary: ProjectSummary,
}

impl HomeState {
    fn new(summary: ProjectSummary) -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        Self { menu_state, summary }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Abre a Home Screen em Ratatui, bloqueia até o usuário selecionar uma ação,
/// restaura o terminal e retorna a ação escolhida.
pub fn run(summary: ProjectSummary) -> Result<HomeAction> {
    let mut state = HomeState::new(summary);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let action = event_loop(&mut terminal, &mut state)?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(action)
}

fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    state: &mut HomeState,
) -> Result<HomeAction> {
    loop {
        terminal.draw(|f| render(f, state))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(HomeAction::Quit),
                    KeyCode::Up => {
                        let i = state.menu_state.selected().unwrap_or(0);
                        if i > 0 {
                            state.menu_state.select(Some(i - 1));
                        }
                    }
                    KeyCode::Down => {
                        let i = state.menu_state.selected().unwrap_or(0);
                        if i + 1 < MENU_ITEMS.len() {
                            state.menu_state.select(Some(i + 1));
                        }
                    }
                    KeyCode::Enter => {
                        return Ok(match state.menu_state.selected().unwrap_or(0) {
                            0 => HomeAction::NewPlan,
                            1 => HomeAction::ContinuePlanning,
                            2 => HomeAction::ExecuteDev,
                            3 => HomeAction::OpenDashboard,
                            _ => HomeAction::Quit,
                        });
                    }
                    KeyCode::Char('1') => return Ok(HomeAction::NewPlan),
                    KeyCode::Char('2') => return Ok(HomeAction::ContinuePlanning),
                    KeyCode::Char('3') => return Ok(HomeAction::ExecuteDev),
                    KeyCode::Char('4') => return Ok(HomeAction::OpenDashboard),
                    KeyCode::Char('5') => return Ok(HomeAction::Quit),
                    _ => {}
                }
            }
        }
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn render(f: &mut Frame, state: &mut HomeState) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // Banner
            Constraint::Min(8),    // Body (Menu + Status)
            Constraint::Length(3), // Footer
        ])
        .split(area);

    render_banner(f, chunks[0]);
    render_body(f, state, chunks[1]);
    render_footer(f, chunks[2]);
}

fn render_banner(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  AI ORCHESTRATOR",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  ─────────────────────────────────────────────────────",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(vec![
            Span::styled(
                "  Multi-Agent Development System  ",
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("│  v0.1.0", Style::default().fg(Color::Gray)),
        ]),
        Line::from(Span::styled(
            "  Arquiteto  ▸  Dev  ▸  Auditor  ▸  Human",
            Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
        )),
        Line::from(""),
    ];

    let banner = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " AI Orchestrator ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center),
    );
    f.render_widget(banner, area);
}

fn render_body(f: &mut Frame, state: &mut HomeState, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    render_menu(f, state, columns[0]);
    render_status(f, state, columns[1]);
}

fn render_menu(f: &mut Frame, state: &mut HomeState, area: Rect) {
    let items: Vec<ListItem> = MENU_ITEMS
        .iter()
        .map(|(label, _)| ListItem::new(format!("  {}", label)))
        .collect();

    let menu = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray))
                .title(Span::styled(
                    " Menu ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(25, 25, 60)),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(menu, area, &mut state.menu_state);
}

fn render_status(f: &mut Frame, state: &mut HomeState, area: Rect) {
    let selected_desc = state
        .menu_state
        .selected()
        .and_then(|i| MENU_ITEMS.get(i))
        .map(|(_, d)| *d)
        .unwrap_or("");

    let s = &state.summary;

    let progress = if s.tasks_total > 0 {
        let pct = (s.tasks_completed * 100) / s.tasks_total;
        let filled = (s.tasks_completed * 20) / s.tasks_total;
        let bar: String = (0..20usize).map(|i| if i < filled { '█' } else { '░' }).collect();
        format!("[{}] {}%  ({}/{})", bar, pct, s.tasks_completed, s.tasks_total)
    } else {
        "Sem tarefas registradas".to_string()
    };

    let last_plan = if s.last_plan_title.is_empty() {
        "Nenhum plano criado ainda".to_string()
    } else {
        format!("{} ({})", s.last_plan_title, s.last_plan_status)
    };

    let last_run = if s.last_run_at.is_empty() {
        "Nenhum run registrado".to_string()
    } else {
        s.last_run_at[..19.min(s.last_run_at.len())].to_string()
    };

    let text = format!(
        "{}\n\n── Projeto ─────────────────────────────\n\n  Último Plano    {}\n  Total de Planos  {}\n\n── Tarefas ──────────────────────────────\n\n  {}\n\n── Último Run ───────────────────────────\n\n  {}",
        selected_desc,
        last_plan,
        s.plans_count,
        progress,
        last_run,
    );

    let status = Paragraph::new(text.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray))
                .title(Span::styled(
                    " Status do Projeto ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(status, area);
}

fn render_footer(f: &mut Frame, area: Rect) {
    let footer = Paragraph::new(
        "  ↑↓: Navegar   Enter: Executar   1-5: Atalho direto   q: Sair",
    )
    .style(Style::default().fg(Color::Gray))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray)),
    );
    f.render_widget(footer, area);
}
