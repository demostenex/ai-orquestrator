use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use inquire::Text;
use ratatui::{backend::CrosstermBackend, prelude::*, widgets::{Block, Borders, Paragraph}};
use std::io;
use std::path::PathBuf;
use tokio::task;

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
}

#[derive(Clone)]
enum SelectorCtx {
    ForDashboard,
    ForContinue,
    ForDev,
}

// Ações que precisam suspender o Ratatui para usar inquire
enum Suspend {
    NewPlan,
    ContinuePlanning { plan_id: String },
    ExecuteDev { plan_id: String },
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
                let rt = tokio::runtime::Runtime::new()?;
                let data = rt.block_on(DashboardState::load(
                    app.orchestrator_dir.clone(),
                    plan_id,
                ))?;
                app.dash_ui = DashboardUiState::new(data.recent_events.len());
                app.dash_data = data;
                app.view = AppView::Dashboard;
            }
            LoopCmd::ReloadDashboard => {
                let rt = tokio::runtime::Runtime::new()?;
                let data = rt.block_on(DashboardState::load(
                    app.orchestrator_dir.clone(),
                    app.dash_data.plan_id.clone(),
                ))?;
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
                SelectorCtx::ForContinue => LoopCmd::Suspend(Suspend::ContinuePlanning { plan_id }),
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

fn render_footer(f: &mut Frame, view: &AppView, area: Rect) {
    let text = match view {
        AppView::Home =>
            "  ↑↓: Navegar   Enter: Executar   1-5: Atalho direto   q: Sair",
        AppView::Selector(_) =>
            "  ↑↓: Navegar   Enter: Selecionar   q/Esc: Voltar ao Menu",
        AppView::Dashboard =>
            "  ↑↓: Feed   Enter: Carregar Handoff   r: Recarregar   q/Esc: Voltar",
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
        Suspend::ContinuePlanning { plan_id } => {
            handle_continue_planning(plan_id).await?;
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
    Db::open_readonly(orchestrator_dir)
        .await
        .ok()
        .map(|db| async move { db.list_plans().await.unwrap_or_default() })
        .map(|f| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(f)
        })
        .unwrap_or_default()
}

async fn handle_continue_planning(plan_id: String) -> Result<()> {
    let cli1 = task::spawn_blocking(|| {
        Text::new("CLI para o Arquiteto (ex: claude, gemini):").prompt()
    })
    .await??;

    let cli2 = task::spawn_blocking(|| {
        Text::new("CLI para o Dev (vazio = mesmo que Arquiteto):")
            .prompt()
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .await?;

    let max_turns = task::spawn_blocking(|| {
        Text::new("Número máximo de turnos:")
            .with_default("10")
            .prompt()
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .unwrap_or(10usize)
    })
    .await?;

    crate::commands::plan::execute(PlanCommands::Continue {
        plan_id: Some(plan_id),
        cli1,
        cli2,
        max_turns,
    })
    .await
}
