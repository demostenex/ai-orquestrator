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
    Executing,
    Gate,
}

#[derive(Clone)]
enum SelectorCtx {
    ForDashboard,
    ForContinue,
    ForDev,
}


// ── Estado de execução em background ─────────────────────────────────────────

struct ExecState {
    title: String,
    lines: Vec<String>,
    gate_content: Option<String>,
    gate_type: String, // "planning" | "diff_review" | "apply" | "inter_task" | "error"
}

impl ExecState {
    fn new(title: String) -> Self {
        Self { title, lines: vec![], gate_content: None, gate_type: String::new() }
    }
}

// ── Prompt: coleta de campos dentro do TUI ────────────────────────────────────

struct PromptField {
    label: &'static str,
    optional: bool,
    default: Option<&'static str>,
}

enum PromptNext {
    ContinuePlanning { plan_id: String },
    NewPlan,
    GateEnrich,
    DevMode { plan_id: String },
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
    fn for_new_plan() -> Self {
        Self {
            title: "Novo Plano",
            fields: vec![
                PromptField { label: "Título do plano", optional: false, default: None },
            ],
            current: 0,
            buffer: String::new(),
            collected: Vec::new(),
            next: PromptNext::NewPlan,
        }
    }

    fn for_dev(plan_id: String) -> Self {
        Self {
            title: "Executar Modo Dev",
            fields: vec![
                PromptField { label: "CLI para a IA Dev (ex: claude, gemini)", optional: false, default: None },
                PromptField { label: "CLI para a Auditora (vazio = aprovação automática)", optional: true, default: None },
            ],
            current: 0,
            buffer: String::new(),
            collected: Vec::new(),
            next: PromptNext::DevMode { plan_id },
        }
    }

    fn for_enrich() -> Self {
        Self {
            title: "Enriquecer Plano",
            fields: vec![
                PromptField { label: "Notas para a IA (instruções de ajuste)", optional: false, default: None },
            ],
            current: 0,
            buffer: String::new(),
            collected: Vec::new(),
            next: PromptNext::GateEnrich,
        }
    }

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
    home_memory: String,
    orchestrator_dir: PathBuf,
    exec: ExecState,
    log_rx: Option<crate::core::stream::LogRx>,
    gate_tx: Option<crate::core::stream::GateTx>,
}

impl TuiApp {
    fn new(orchestrator_dir: PathBuf, summary: ProjectSummary, plans: Vec<PlanSummary>, home_memory: String) -> Self {
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
            home_memory,
            orchestrator_dir,
            exec: ExecState::new(String::new()),
            log_rx: None,
            gate_tx: None,
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn start() -> Result<()> {
    let config = crate::core::config::Config::load()?;
    let orchestrator_dir = config.orchestrator_dir.clone();

    let summary = Db::get_home_summary(orchestrator_dir.clone()).await.unwrap_or_default();
    let plans = load_plans(&orchestrator_dir).await;
    let home_memory = load_ai_memory_feed();

    let mut app = TuiApp::new(orchestrator_dir.clone(), summary, plans, home_memory);

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

        // Drena o canal de log (execução em background)
        if matches!(app.view, AppView::Executing | AppView::Gate) {
            if let Some(ref rx) = app.log_rx {
                loop {
                    match rx.try_recv() {
                        Ok(crate::core::stream::LogEvent::Line(s)) => {
                            app.exec.lines.push(s);
                            // mantém só as últimas 200 linhas
                            if app.exec.lines.len() > 200 {
                                app.exec.lines.drain(..app.exec.lines.len() - 200);
                            }
                        }
                        Ok(crate::core::stream::LogEvent::GateNeeded { content, gate_type }) => {
                            app.exec.gate_content = Some(content);
                            app.exec.gate_type = gate_type;
                            app.view = AppView::Gate;
                            break;
                        }
                        Ok(crate::core::stream::LogEvent::Done) => {
                            app.log_rx = None;
                            app.gate_tx = None;
                            app.summary = Db::get_home_summary(app.orchestrator_dir.clone()).await.unwrap_or_default();
                            app.plans = load_plans(&app.orchestrator_dir).await;
                            app.home_memory = load_ai_memory_feed();
                            app.view = AppView::Home;
                            break;
                        }
                        Ok(crate::core::stream::LogEvent::Failed(e)) => {
                            app.exec.lines.push(format!("ERRO: {}", e));
                            app.log_rx = None;
                            app.gate_tx = None;
                            app.view = AppView::Executing; // mantém na tela para o user ver o erro
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            app.log_rx = None;
                            app.gate_tx = None;
                            // Fix 2: não vai para Home se há erros — usuário precisa ler
                            let has_errors = app.exec.lines.iter()
                                .any(|l| l.starts_with("ERRO:") || l.contains("❌") || l.contains("ERRO"));
                            if has_errors {
                                app.exec.lines.push(
                                    "(thread finalizada — leia os erros acima e pressione q)".to_string()
                                );
                                // permanece em Executing
                            } else {
                                app.view = AppView::Home;
                            }
                            break;
                        }
                    }
                }
            }
        }

        match dispatch_key(app, key.code) {
            LoopCmd::Continue => {}
            LoopCmd::Quit => break,
            LoopCmd::GoTo(view) => {
                app.view = view;
            }
            LoopCmd::StartPlanning { plan_id, cli1, cli2, max_turns } => {
                use crate::core::stream::*;
                let (log_tx, log_rx) = std::sync::mpsc::sync_channel::<LogEvent>(256);
                let (gate_tx, gate_rx) = std::sync::mpsc::sync_channel::<GateDecision>(1);
                let dir = app.orchestrator_dir.clone();
                std::thread::spawn(move || {
                    match tokio::runtime::Runtime::new() {
                        Err(e) => { let _ = log_tx.send(LogEvent::Failed(format!("runtime: {}", e))); }
                        Ok(rt) => {
                            let result = rt.block_on(run_planning_background(
                                dir, plan_id, cli1, cli2, max_turns, log_tx.clone(), gate_rx,
                            ));
                            if let Err(e) = result { let _ = log_tx.send(LogEvent::Failed(e.to_string())); }
                        }
                    }
                });
                app.exec = ExecState::new("Planejamento em andamento...".to_string());
                app.log_rx = Some(log_rx);
                app.gate_tx = Some(gate_tx);
                app.view = AppView::Executing;
            }
            LoopCmd::StartDev { plan_id, cli_dev, cli_audit } => {
                use crate::core::stream::*;
                let (log_tx, log_rx) = std::sync::mpsc::sync_channel::<LogEvent>(256);
                let (gate_tx, gate_rx) = std::sync::mpsc::sync_channel::<GateDecision>(1);
                std::thread::spawn(move || {
                    match tokio::runtime::Runtime::new() {
                        Err(e) => { let _ = log_tx.send(LogEvent::Failed(format!("runtime: {}", e))); }
                        Ok(rt) => {
                            let result = rt.block_on(crate::commands::run::execute_tui(
                                plan_id, cli_dev, cli_audit, log_tx.clone(), gate_rx,
                            ));
                            if let Err(e) = result { let _ = log_tx.send(LogEvent::Failed(e.to_string())); }
                        }
                    }
                });
                app.exec = ExecState::new("Modo Dev em andamento...".to_string());
                app.log_rx = Some(log_rx);
                app.gate_tx = Some(gate_tx);
                app.view = AppView::Executing;
            }
            LoopCmd::StartScan => {
                use crate::core::stream::*;

                // Fix 3: valida se o binário ai-memory existe no PATH antes de spawnar
                let cli_ok = std::process::Command::new("ai-memory")
                    .arg("status")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok();

                if !cli_ok {
                    let mut exec = ExecState::new("🔍 Deep Scan — Erro de Configuração".to_string());
                    exec.lines.push("ERRO: 'ai-memory' não encontrado no PATH.".to_string());
                    exec.lines.push("Instale o ai-memory CLI e tente novamente.".to_string());
                    exec.lines.push("(pressione q para voltar ao menu)".to_string());
                    app.exec = exec;
                    app.view = AppView::Executing;
                } else {
                    let (log_tx, log_rx) = std::sync::mpsc::sync_channel::<LogEvent>(512);
                    let (gate_tx, gate_rx) = std::sync::mpsc::sync_channel::<GateDecision>(1);
                    let dir = app.orchestrator_dir.clone();
                    // Fix 1: Runtime::new() sem unwrap — erro propagado pelo canal
                    std::thread::spawn(move || {
                        match tokio::runtime::Runtime::new() {
                            Err(e) => {
                                let _ = log_tx.send(LogEvent::Failed(
                                    format!("Falha ao criar runtime tokio: {}", e)
                                ));
                            }
                            Ok(rt) => {
                                let result = rt.block_on(run_deep_scan(dir, log_tx.clone(), gate_rx));
                                if let Err(e) = result {
                                    let _ = log_tx.send(LogEvent::Failed(e.to_string()));
                                }
                            }
                        }
                    });
                    app.exec = ExecState::new("🔍 Deep Scan — Sincronizando ai-memory".to_string());
                    app.log_rx = Some(log_rx);
                    app.gate_tx = Some(gate_tx);
                    app.view = AppView::Executing;
                }
            }
            LoopCmd::CreatePlan { title } => {
                create_plan_native(&app.orchestrator_dir, title).await?;
                app.summary = Db::get_home_summary(app.orchestrator_dir.clone()).await.unwrap_or_default();
                app.plans = load_plans(&app.orchestrator_dir).await;
                app.view = AppView::Home;
            }
            LoopCmd::GateDecide(decision) => {
                if let Some(ref tx) = app.gate_tx {
                    let _ = tx.send(decision);
                }
                app.exec.gate_content = None;
                app.view = AppView::Executing;
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
    StartPlanning { plan_id: String, cli1: String, cli2: Option<String>, max_turns: usize },
    StartDev { plan_id: String, cli_dev: String, cli_audit: Option<String> },
    StartScan,
    CreatePlan { title: String },
    GateDecide(crate::core::stream::GateDecision),
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
        AppView::Executing => dispatch_executing(key),
        AppView::Gate => dispatch_gate(key),
    }
}

fn dispatch_home(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::Quit,
        KeyCode::Char('r') => {
            app.home_memory = load_ai_memory_feed();
            LoopCmd::Continue
        }
        KeyCode::Up => { app.home.move_up(); LoopCmd::Continue }
        KeyCode::Down => { app.home.move_down(); LoopCmd::Continue }
        KeyCode::Enter | KeyCode::Char('1'..='6') => {
            let idx = match key {
                KeyCode::Char(c @ '1'..='6') => (c as usize) - ('1' as usize),
                _ => app.home.selected(),
            };
            match idx {
                0 => LoopCmd::GoTo(AppView::Prompt(PromptState::for_new_plan())),
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
                4 => LoopCmd::StartScan,
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
                SelectorCtx::ForDev => LoopCmd::GoTo(AppView::Prompt(PromptState::for_dev(plan_id))),
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
                    LoopCmd::StartPlanning { plan_id, cli1, cli2, max_turns }
                }
                PromptNext::NewPlan => {
                    LoopCmd::CreatePlan { title: collected[0].clone() }
                }
                PromptNext::GateEnrich => {
                    use crate::core::stream::GateDecision;
                    LoopCmd::GateDecide(GateDecision::Enrich(collected[0].clone()))
                }
                PromptNext::DevMode { plan_id } => {
                    let cli_dev = collected[0].clone();
                    let cli_audit = if collected[1].is_empty() { None } else { Some(collected[1].clone()) };
                    LoopCmd::StartDev { plan_id: plan_id.clone(), cli_dev, cli_audit }
                }
            }
        }
        _ => LoopCmd::Continue,
    }
}

fn dispatch_executing(key: KeyCode) -> LoopCmd {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::GoTo(AppView::Home),
        _ => LoopCmd::Continue,
    }
}

fn dispatch_gate(key: KeyCode) -> LoopCmd {
    use crate::core::stream::GateDecision;
    match key {
        KeyCode::Char('c') | KeyCode::Enter => LoopCmd::GateDecide(GateDecision::Continue),
        KeyCode::Char('e') => LoopCmd::GoTo(AppView::Prompt(PromptState::for_enrich())),
        KeyCode::Char('f') => LoopCmd::GateDecide(GateDecision::Finalize),
        KeyCode::Char('a') | KeyCode::Esc => LoopCmd::GateDecide(GateDecision::Abort),
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
        AppView::Home => home::render(f, &mut app.home, &app.summary, &app.home_memory, chunks[1]),
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
        AppView::Executing => render_executing(f, &app.exec, chunks[1]),
        AppView::Gate => render_gate(f, &app.exec, chunks[1]),
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

fn render_executing(f: &mut Frame, exec: &ExecState, area: Rect) {
    let visible: Vec<&str> = exec.lines.iter().rev().take(30).rev().map(|s| s.as_str()).collect();
    let text = visible.join("\n");
    let panel = Paragraph::new(text.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    format!(" ⠋ {} ", exec.title),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(panel, area);
}

fn render_gate(f: &mut Frame, exec: &ExecState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(6)])
        .split(area);

    // Preview do conteúdo do plano
    let preview = exec.gate_content.as_deref().unwrap_or("");
    let preview_text: String = preview.lines().take(20).collect::<Vec<_>>().join("\n");
    let plan_panel = Paragraph::new(preview_text.as_str())
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title(" Plano Gerado ").title_alignment(Alignment::Center))
        .wrap(Wrap { trim: true });
    f.render_widget(plan_panel, chunks[0]);

    // Gate de decisão
    let gate_options = match exec.gate_type.as_str() {
        "planning" => "\n  [C] / Enter  →  Continuar (próximo turno)\n  [E]          →  Enriquecer (adicionar notas)\n  [F]          →  Finalizar planejamento\n  [A] / Esc    →  Abortar",
        "diff_review" => "\n  [C] / Enter  →  Aprovar diff\n  [E]          →  Enriquecer (notas para o Dev)\n  [A] / Esc    →  Rejeitar diff",
        "apply" => "\n  [C] / Enter  →  Aplicar patch ao workspace\n  [A] / Esc    →  Pular (não aplicar)",
        "inter_task" => "\n  [C] / Enter  →  Próxima tarefa\n  [E]          →  Repetir com notas\n  [A] / Esc    →  Encerrar Dev Mode",
        "error" => "\n  [C] / Enter  →  Continuar mesmo assim (cuidado)\n  [A] / Esc    →  Abortar tarefa",
        _ => "\n  [C] / Enter  →  Continuar\n  [A] / Esc    →  Abortar",
    };
    let gate = Paragraph::new(gate_options)
    .style(Style::default().fg(Color::Yellow))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(Span::styled(
                " ✋ Portão — Decisão Necessária ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center),
    );
    f.render_widget(gate, chunks[1]);
}

fn render_footer(f: &mut Frame, view: &AppView, area: Rect) {
    let text = match view {
        AppView::Home =>
            "  ↑↓: Navegar   Enter: Executar   1-6: Atalho   r: Recarregar Memória   q: Sair",
        AppView::Selector(_) =>
            "  ↑↓: Navegar   Enter: Selecionar   q/Esc: Voltar ao Menu",
        AppView::Dashboard =>
            "  ↑↓: Feed   Enter: Carregar Handoff   r: Recarregar   q/Esc: Voltar",
        AppView::Prompt(_) =>
            "  Enter: Confirmar   Backspace: Apagar   Esc: Cancelar",
        AppView::Executing =>
            "  q: Voltar ao Menu (planejamento continua em background)",
        AppView::Gate =>
            "  C/Enter: Continuar   E: Enriquecer   F: Finalizar   A/Esc: Abortar",
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


// ── Helpers ───────────────────────────────────────────────────────────────────

async fn load_plans(orchestrator_dir: &PathBuf) -> Vec<PlanSummary> {
    let Ok(db) = Db::open_readonly(orchestrator_dir).await else {
        return vec![];
    };
    db.list_plans().await.unwrap_or_default()
}

/// Carrega as notas e páginas recentes do ai-memory via CLI.
fn load_ai_memory_feed() -> String {
    // Tenta buscar as páginas mais recentes
    for args in [
        vec!["recent", "--limit", "5"],
        vec!["recent"],
        vec!["list", "--recent"],
    ] {
        if let Ok(output) = std::process::Command::new("ai-memory").args(&args).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if !text.trim().is_empty() {
                    // Limita a 40 linhas para caber no painel
                    let trimmed: String = text.lines().take(40).collect::<Vec<_>>().join("\n");
                    return trimmed;
                }
            }
        }
    }
    "── Feed de Conhecimento ──\n\nai-memory não disponível\nou sem páginas recentes.\n\nPressione 'r' para tentar novamente.".to_string()
}

/// Cria um novo plano diretamente via DB, sem sair do raw mode.
async fn create_plan_native(orchestrator_dir: &PathBuf, title: String) -> Result<()> {
    use uuid::Uuid;
    let config = crate::core::config::Config::load()?;
    let plan_id = Uuid::new_v4().to_string();
    let run_id = Uuid::new_v4().to_string();
    let db = crate::core::db::Db::open(
        orchestrator_dir,
        &config.workspace_dir,
        &run_id,
        &config.step_id,
        "planning",
        "HEAD",
        "",
        "",
    )?;
    db.create_plan(&plan_id, &title).await?;
    crate::commands::export_plan_to_markdown(orchestrator_dir, &db, &plan_id).await?;
    Ok(())
}

// ── Deep Scan (Fase 5.6.2) ────────────────────────────────────────────────────

struct MemoryPageInfo {
    path: String,
    title: String,
}

/// Lista todas as páginas do ai-memory via `search` CLI.
/// O CLI retorna texto no formato:
///   `  <path>  rank=<float>\n    <title>\n    <snippet>...`
/// Usa múltiplas queries comuns para maximizar cobertura.
fn list_ai_memory_pages() -> Vec<MemoryPageInfo> {
    let mut seen = std::collections::HashSet::new();
    let mut pages = Vec::new();

    // Queries amplas para capturar a maioria das páginas
    for query in ["e", "a", "o", "i"] {
        let Ok(output) = std::process::Command::new("ai-memory")
            .args(["search", query, "--limit", "500"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        else { continue };

        let text = String::from_utf8_lossy(&output.stdout);
        let mut last_path: Option<String> = None;

        for line in text.lines() {
            // Path line: "  <path>  rank=<float>" (exactly 2 leading spaces, contains rank=)
            let stripped = line.strip_prefix("  ").unwrap_or("");
            if stripped.starts_with("  ") || stripped.is_empty() {
                // Title line (4 spaces) or blank — capture title for last path
                if let Some(ref p) = last_path {
                    if !stripped.trim().is_empty() && !stripped.contains('<') {
                        let title = stripped.trim().to_string();
                        if seen.insert(p.clone()) {
                            pages.push(MemoryPageInfo { path: p.clone(), title });
                        }
                        last_path = None;
                    }
                }
                continue;
            }
            // Check if this looks like a path line
            if let Some(path_part) = stripped.split("  rank=").next() {
                let path = path_part.trim().to_string();
                if path.contains('/') && path.ends_with(".md") {
                    last_path = Some(path);
                    continue;
                }
            }
            last_path = None;
        }
    }

    pages
}

/// Extrai event_type e run_id a partir do path da página wiki.
fn extract_metadata_from_path(path: &str) -> (&'static str, String) {
    let run_id_from_path = |s: &str| -> String {
        // handoffs/run_<id>/... → extrai <id>
        s.strip_prefix("run_")
            .map(|r| r.split('/').next().unwrap_or(r).strip_suffix(".md").unwrap_or(r).to_string())
            .unwrap_or_else(|| format!("scan-{:016x}",
                path.bytes().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))))
    };

    if path.starts_with("handoffs/") {
        let seg = path.strip_prefix("handoffs/").unwrap_or(path);
        ("handoff_created", run_id_from_path(seg))
    } else if path.starts_with("decisions/") {
        ("gate_passed", format!("scan-decisions-{:016x}",
            path.bytes().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))))
    } else if path.starts_with("plans/") || path.starts_with("notes/") {
        ("gate_enriched", format!("scan-notes-{:016x}",
            path.bytes().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))))
    } else {
        ("handoff_created", format!("scan-other-{:016x}",
            path.bytes().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))))
    }
}

async fn run_deep_scan(
    orchestrator_dir: PathBuf,
    log_tx: crate::core::stream::LogTx,
    _gate_rx: crate::core::stream::GateRx,
) -> anyhow::Result<()> {
    use crate::core::stream::LogEvent;
    use crate::core::db::EventType;

    let send = |msg: String| { let _ = log_tx.send(LogEvent::Line(msg)); };

    send("🔍 Listando páginas do ai-memory...".to_string());
    let pages = list_ai_memory_pages();

    if pages.is_empty() {
        send("⚠ Nenhuma página encontrada. Verifique se o ai-memory CLI está disponível.".to_string());
        let _ = log_tx.send(LogEvent::Done);
        return Ok(());
    }
    send(format!("📄 {} páginas encontradas. Iniciando ingestão...", pages.len()));

    let config = crate::core::config::Config::load()?;
    let db = crate::core::db::Db::open(
        &orchestrator_dir,
        &config.workspace_dir,
        &format!("deep-scan-{}", chrono::Utc::now().timestamp()),
        "deep-scan",
        "scan",
        "HEAD", "", "",
    )?;

    let mut ingested = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for page in &pages {
        let (event_type_str, run_id) = extract_metadata_from_path(&page.path);
        let event_type = match event_type_str {
            "handoff_created" => EventType::HandoffCreated,
            "gate_passed"     => EventType::GatePassed,
            _                 => EventType::GateEnriched,
        };

        match db.reconstruct_event(&run_id, event_type, &page.title, &page.path).await {
            Ok(true)  => { ingested += 1; send(format!("  ✅ {}", page.path)); }
            Ok(false) => { skipped += 1; }
            Err(e)    => { failed += 1; send(format!("  ❌ {}: {}", page.path, e)); }
        }
    }

    send(format!(
        "\n✅ Deep Scan concluído!\n   Ingeridos: {}\n   Já existiam: {}\n   Erros: {}",
        ingested, skipped, failed
    ));
    let _ = log_tx.send(LogEvent::Done);
    Ok(())
}

/// Executa o loop de planejamento em background (chamado de std::thread com runtime próprio).
async fn run_planning_background(
    orchestrator_dir: PathBuf,
    plan_id: String,
    cli1: String,
    cli2: Option<String>,
    max_turns: usize,
    log_tx: crate::core::stream::LogTx,
    gate_rx: crate::core::stream::GateRx,
) -> anyhow::Result<()> {
    use crate::commands::plan::run_planning_loop;
    use crate::core::db::Db;
    use uuid::Uuid;

    let config = crate::core::config::Config::load()?;

    let agents_owned: Vec<(String, String, String)> = if let Some(ref cli_dev) = cli2 {
        vec![
            ("architect".to_string(), "Arquiteto".to_string(), cli1.clone()),
            ("dev".to_string(), "Dev".to_string(), cli_dev.clone()),
        ]
    } else {
        vec![("architect".to_string(), "Arquiteto".to_string(), cli1.clone())]
    };
    let agents: Vec<(&str, &str, &str)> = agents_owned
        .iter()
        .map(|(a, r, c)| (a.as_str(), r.as_str(), c.as_str()))
        .collect();

    let db = Db::open(
        &orchestrator_dir,
        &config.workspace_dir,
        &Uuid::new_v4().to_string(),
        &config.step_id,
        "planning",
        "HEAD",
        "",
        "",
    )?;

    run_planning_loop(&db, &orchestrator_dir, &plan_id, &agents, max_turns, Some(log_tx.clone()), Some(gate_rx)).await?;

    let _ = log_tx.send(crate::core::stream::LogEvent::Done);
    Ok(())
}

