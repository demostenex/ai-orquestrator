use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, prelude::*, widgets::{Block, Borders, Paragraph, Wrap}};
use std::io;
use std::path::PathBuf;

pub mod dashboard;
pub mod home;
pub mod plan_selector;

use crate::cli::PlanCommands;
use crate::core::db::{Db, ProjectSummary};
use crate::schemas::PlanSummary;
use dashboard::{DashboardCmd, DashboardState, DashboardUiState};
use home::HomeState;
use plan_selector::SelectorState;

// ── View State Machine ────────────────────────────────────────────────────────

enum AppView {
    Home,
    Selector(SelectorCtx),
    Dashboard,
    Prompt(PromptState),
}

#[derive(Clone)]
enum SelectorCtx {
    ForDashboard,
    ForContinue,
    ForDev,
}

// Ações que precisam suspender o Ratatui (executar comando externo com output)
enum Suspend {
    NewPlan,
    ContinuePlanning { plan_id: String, cli1: String, cli2: Option<String>, max_turns: usize },
    ExecuteDev { plan_id: String },
}

// ── Prompt: coleta de campos dentro do TUI ────────────────────────────────────

struct PromptField {
    label: &'static str,
    optional: bool,
    default: Option<&'static str>,
}

enum PromptNext {
    ContinuePlanning { plan_id: String },
}

struct PromptState {
    title: &'static str,
    fields: Vec<PromptField>,
    current: usize,
    buffer: String,
    collected: Vec<String>,
    next: PromptNext,
}

impl PromptState {
    fn for_continue(plan_id: String) -> Self {
        Self {
            title: "Continuar Planejamento",
            fields: vec![
                PromptField { label: "CLI para o Arquiteto (ex: claude, gemini)", optional: false, default: None },
                PromptField { label: "CLI para o Dev (vazio = mesmo que Arquiteto)", optional: true, default: None },
                PromptField { label: "Máximo de turnos", optional: false, default: Some("10") },
            ],
            current: 0,
            buffer: String::new(),
            collected: Vec::new(),
            next: PromptNext::ContinuePlanning { plan_id },
        }
    }

    fn progress(&self) -> String {
        format!("{}/{}", self.current + 1, self.fields.len())
    }
}

// ── App State ─────────────────────────────────────────────────────────────────

struct TuiApp {
    view: AppView,
    home: HomeState,
    selector: SelectorState,
    selector_ctx: SelectorCtx,
    dash_data: DashboardState,
    dash_ui: DashboardUiState,
    plans: Vec<PlanSummary>,
    summary: ProjectSummary,
    orchestrator_dir: PathBuf,
}

impl TuiApp {
    fn new(orchestrator_dir: PathBuf, summary: ProjectSummary, plans: Vec<PlanSummary>) -> Self {
        let selector = SelectorState::new(plans.len());
        Self {
            view: AppView::Home,
            home: HomeState::new(),
            selector,
            selector_ctx: SelectorCtx::ForDashboard,
            dash_data: DashboardState::default(),
            dash_ui: DashboardUiState::new(0),
            plans,
            summary,
            orchestrator_dir,
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn start() -> Result<()> {
    let config = crate::core::config::Config::load()?;
    let orchestrator_dir = config.orchestrator_dir.clone();

    let summary = Db::get_home_summary(orchestrator_dir.clone()).await.unwrap_or_default();
    let plans = load_plans(&orchestrator_dir).await;

    let mut app = TuiApp::new(orchestrator_dir.clone(), summary, plans);

    // Setup terminal — único ponto de enable/disable em toda a sessão
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

// ── Loop principal ────────────────────────────────────────────────────────────

type AppTerminal = Terminal<CrosstermBackend<io::Stdout>>;

async fn run_loop(terminal: &mut AppTerminal, app: &mut TuiApp) -> Result<()> {
    loop {
        terminal.draw(|f| render_app(f, app))?;

        if !event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match dispatch_key(app, key.code) {
            LoopCmd::Continue => {}
            LoopCmd::Quit => break,
            LoopCmd::GoTo(view) => {
                app.view = view;
            }
            LoopCmd::Suspend(action) => {
                suspend_for_input(terminal, app, action).await?;
            }
            LoopCmd::LoadDashboard(plan_id) => {
                let data = DashboardState::load(app.orchestrator_dir.clone(), plan_id).await?;
                app.dash_ui = DashboardUiState::new(data.recent_events.len());
                app.dash_data = data;
                app.view = AppView::Dashboard;
            }
            LoopCmd::ReloadDashboard => {
                let data = DashboardState::load(
                    app.orchestrator_dir.clone(),
                    app.dash_data.plan_id.clone(),
                )
                .await?;
                app.dash_ui.reset(data.recent_events.len());
                app.dash_data = data;
            }
            LoopCmd::LoadMemory(run_id) => {
                app.dash_ui.memory_content = Some(dashboard::load_memory_for_run(&run_id));
            }
        }
    }
    Ok(())
}

// ── Dispatch de teclas por view ───────────────────────────────────────────────

enum LoopCmd {
    Continue,
    Quit,
    GoTo(AppView),
    Suspend(Suspend),
    LoadDashboard(String),
    ReloadDashboard,
    LoadMemory(String),
}

fn dispatch_key(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match &app.view {
        AppView::Home => dispatch_home(app, key),
        AppView::Selector(_) => dispatch_selector(app, key),
        AppView::Dashboard => dispatch_dashboard(app, key),
        AppView::Prompt(_) => dispatch_prompt(app, key),
    }
}

fn dispatch_home(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::Quit,
        KeyCode::Up => { app.home.move_up(); LoopCmd::Continue }
        KeyCode::Down => { app.home.move_down(); LoopCmd::Continue }
        KeyCode::Enter | KeyCode::Char('1'..='5') => {
            let idx = match key {
                KeyCode::Char(c @ '1'..='5') => (c as usize) - ('1' as usize),
                _ => app.home.selected(),
            };
            match idx {
                0 => LoopCmd::Suspend(Suspend::NewPlan),
                1 => {
                    app.selector = SelectorState::new(app.plans.len());
                    app.selector_ctx = SelectorCtx::ForContinue;
                    LoopCmd::GoTo(AppView::Selector(SelectorCtx::ForContinue))
                }
                2 => {
                    app.selector = SelectorState::new(app.plans.len());
                    app.selector_ctx = SelectorCtx::ForDev;
                    LoopCmd::GoTo(AppView::Selector(SelectorCtx::ForDev))
                }
                3 => {
                    app.selector = SelectorState::new(app.plans.len());
                    app.selector_ctx = SelectorCtx::ForDashboard;
                    LoopCmd::GoTo(AppView::Selector(SelectorCtx::ForDashboard))
                }
                _ => LoopCmd::Quit,
            }
        }
        _ => LoopCmd::Continue,
    }
}

fn dispatch_selector(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::GoTo(AppView::Home),
        KeyCode::Up => { app.selector.move_up(); LoopCmd::Continue }
        KeyCode::Down => { app.selector.move_down(app.plans.len()); LoopCmd::Continue }
        KeyCode::Enter => {
            let Some(plan) = app.selector.pick(&app.plans) else {
                return LoopCmd::Continue;
            };
            let plan_id = plan.id.clone();
            let ctx = app.selector_ctx.clone();
            match ctx {
                SelectorCtx::ForDashboard => LoopCmd::LoadDashboard(plan_id),
                SelectorCtx::ForContinue => LoopCmd::GoTo(AppView::Prompt(PromptState::for_continue(plan_id))),
                SelectorCtx::ForDev => LoopCmd::Suspend(Suspend::ExecuteDev { plan_id }),
            }
        }
        _ => LoopCmd::Continue,
    }
}

fn dispatch_dashboard(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    use crossterm::event::KeyEvent;
    let fake_key = KeyEvent::new(key, crossterm::event::KeyModifiers::NONE);
    match dashboard::handle_key(fake_key, &mut app.dash_ui, &app.dash_data) {
        DashboardCmd::None => LoopCmd::Continue,
        DashboardCmd::Back => LoopCmd::GoTo(AppView::Home),
        DashboardCmd::Reload => LoopCmd::ReloadDashboard,
        DashboardCmd::LoadMemory(run_id) => LoopCmd::LoadMemory(run_id),
    }
}

fn dispatch_prompt(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    let AppView::Prompt(ref mut ps) = app.view else { return LoopCmd::Continue };
    match key {
        KeyCode::Esc => return LoopCmd::GoTo(AppView::Home),
        KeyCode::Backspace => { ps.buffer.pop(); return LoopCmd::Continue; }
        KeyCode::Char(c) => { ps.buffer.push(c); return LoopCmd::Continue; }
        KeyCode::Enter => {
            // Aplica default se buffer vazio e campo tem default
            if ps.buffer.is_empty() {
                if let Some(def) = ps.fields[ps.current].default {
                    ps.buffer = def.to_string();
                }
            }
            ps.collected.push(ps.buffer.clone());
            ps.buffer.clear();
            ps.current += 1;

            if ps.current < ps.fields.len() {
                return LoopCmd::Continue; // mais campos a coletar
            }

            // Todos os campos coletados — constrói o Suspend
            let collected = ps.collected.clone();
            match &ps.next {
                PromptNext::ContinuePlanning { plan_id } => {
                    let plan_id = plan_id.clone();
                    let cli1 = collected[0].clone();
                    let cli2 = if collected[1].is_empty() { None } else { Some(collected[1].clone()) };
                    let max_turns: usize = collected[2].parse().unwrap_or(10);
                    LoopCmd::Suspend(Suspend::ContinuePlanning { plan_id, cli1, cli2, max_turns })
                }
            }
        }
        _ => LoopCmd::Continue,
    }
}

// ── Render global ─────────────────────────────────────────────────────────────

fn render_app(f: &mut Frame<'_>, app: &mut TuiApp) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Banner persistente
            Constraint::Min(5),    // Corpo dinâmico
            Constraint::Length(3), // Footer contextual
        ])
        .split(area);

    render_banner(f, chunks[0]);

    match &app.view {
        AppView::Home => home::render(f, &mut app.home, &app.summary, chunks[1]),
        AppView::Selector(ctx) => {
            let label = match ctx {
                SelectorCtx::ForDashboard => "Selecionar plano para abrir no Dashboard",
                SelectorCtx::ForContinue => "Selecionar plano para continuar o planejamento",
                SelectorCtx::ForDev => "Selecionar plano para Executar Modo Dev",
            };
            plan_selector::render(f, &mut app.selector, &app.plans, label, chunks[1]);
        }
        AppView::Dashboard => {
            dashboard::render_body(f, &app.dash_data, &mut app.dash_ui, chunks[1]);
        }
        AppView::Prompt(ps) => render_prompt(f, ps, chunks[1]),
    }

    render_footer(f, &app.view, chunks[2]);
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
            Span::styled("  Multi-Agent Development System  ", Style::default().fg(Color::Yellow)),
            Span::styled("│  v0.1.0", Style::default().fg(Color::Gray)),
        ]),
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

fn render_prompt(f: &mut Frame, ps: &PromptState, area: Rect) {
    let mut lines: Vec<Line> = vec![Line::from("")];

    // Campos já coletados (verde)
    for (i, field) in ps.fields[..ps.current].iter().enumerate() {
        let val = &ps.collected[i];
        let display = if val.is_empty() { "(padrão)".to_string() } else { val.clone() };
        lines.push(Line::from(vec![
            Span::styled("  ✓ ", Style::default().fg(Color::Green)),
            Span::styled(field.label, Style::default().fg(Color::Gray)),
            Span::styled(format!(": {}", display), Style::default().fg(Color::Green)),
        ]));
    }

    if ps.current < ps.fields.len() {
        let field = &ps.fields[ps.current];
        lines.push(Line::from(""));
        // Label do campo atual
        let suffix = if field.optional { " (opcional)" } else { "" };
        let def_hint = field.default.map(|d| format!(" [padrão: {}]", d)).unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!("  ▶ {}{}{}", field.label, suffix, def_hint),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        // Buffer de input com cursor
        lines.push(Line::from(vec![
            Span::styled("    > ", Style::default().fg(Color::Cyan)),
            Span::styled(ps.buffer.clone(), Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Cyan)),
        ]));
    }

    let text: Vec<&Line> = lines.iter().collect();
    let content = Text::from(lines.clone());
    let panel = Paragraph::new(content)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    format!(" {} ({}) ", ps.title, ps.progress()),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        )
        .wrap(Wrap { trim: false });
    let _ = text; // suprime warning
    f.render_widget(panel, area);
}

fn render_footer(f: &mut Frame, view: &AppView, area: Rect) {
    let text = match view {
        AppView::Home =>
            "  ↑↓: Navegar   Enter: Executar   1-5: Atalho direto   q: Sair",
        AppView::Selector(_) =>
            "  ↑↓: Navegar   Enter: Selecionar   q/Esc: Voltar ao Menu",
        AppView::Dashboard =>
            "  ↑↓: Feed   Enter: Carregar Handoff   r: Recarregar   q/Esc: Voltar",
        AppView::Prompt(_) =>
            "  Enter: Confirmar   Backspace: Apagar   Esc: Cancelar",
    };
    let footer = Paragraph::new(text)
        .style(Style::default().fg(Color::Gray))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
    f.render_widget(footer, area);
}

// ── Suspensão para inputs via inquire ─────────────────────────────────────────

async fn suspend_for_input(
    terminal: &mut AppTerminal,
    app: &mut TuiApp,
    action: Suspend,
) -> Result<()> {
    // Sai do modo raw temporariamente
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    match action {
        Suspend::NewPlan => {
            crate::commands::plan::execute(PlanCommands::New { title: None }).await?;
        }
        Suspend::ContinuePlanning { plan_id, cli1, cli2, max_turns } => {
            crate::commands::plan::execute(PlanCommands::Continue {
                plan_id: Some(plan_id),
                cli1,
                cli2,
                max_turns,
            }).await?;
        }
        Suspend::ExecuteDev { plan_id } => {
            crate::commands::run::execute(None, false, false, false, Some(plan_id)).await?;
        }
    }

    // Recarrega dados e volta ao home
    app.summary = Db::get_home_summary(app.orchestrator_dir.clone()).await.unwrap_or_default();
    app.plans = load_plans(&app.orchestrator_dir).await;
    app.selector = SelectorState::new(app.plans.len());
    app.view = AppView::Home;

    // Re-entra no modo raw
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn load_plans(orchestrator_dir: &PathBuf) -> Vec<PlanSummary> {
    let Ok(db) = Db::open_readonly(orchestrator_dir).await else {
        return vec![];
    };
    db.list_plans().await.unwrap_or_default()
}

