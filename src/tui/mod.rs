use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::io;
use std::path::{Path, PathBuf};

pub mod agent_terminal;
pub mod dashboard;
pub mod home;
pub mod memory;
pub mod plan_selector;

use crate::core::db::{Db, ProjectSummary};
use crate::schemas::PlanSummary;
use agent_terminal::{classify_agent_line, AgentTerminal, AgentTerminalRole};
use dashboard::{DashboardCmd, DashboardState, DashboardUiState};
use home::HomeState;
use memory::MemoryViewState;
use plan_selector::SelectorState;

// ── View State Machine ────────────────────────────────────────────────────────

enum AppView {
    Home,
    Selector(SelectorCtx),
    Dashboard,
    Memory,
    Prompt(PromptState),
    Executing,
    Gate,
}

#[derive(Clone)]
#[allow(clippy::enum_variant_names)]
enum SelectorCtx {
    ForDashboard,
    ForContinue,
    ForDev,
}

// ── Estado de execução em background ─────────────────────────────────────────

struct ExecState {
    title: String,
    lines: Vec<String>,
    architect_terminal: AgentTerminal,
    reviewer_terminal: AgentTerminal,
    dev_terminal: AgentTerminal,
    auditor_terminal: AgentTerminal,
    active_role: Option<AgentTerminalRole>,
    focused_role: Option<AgentTerminalRole>,
    has_planning_reviewer: bool,
    split_panes: bool,
    gate_content: Option<String>,
    gate_type: String, // "planning" | "diff_review" | "apply" | "inter_task" | "error"
}

impl ExecState {
    fn new(title: String) -> Self {
        Self {
            title,
            lines: vec![],
            architect_terminal: AgentTerminal::new("Arquiteto", Color::Yellow),
            reviewer_terminal: AgentTerminal::new("Revisor", Color::LightGreen),
            dev_terminal: AgentTerminal::new("Dev", Color::Cyan),
            auditor_terminal: AgentTerminal::new("Auditora", Color::Magenta),
            active_role: None,
            focused_role: None,
            has_planning_reviewer: false,
            split_panes: false,
            gate_content: None,
            gate_type: String::new(),
        }
    }

    fn push_line(&mut self, line: String) {
        let role = classify_agent_line(&line);
        if let Some(role) = role {
            self.active_role = Some(role);
            if self.focused_role.is_none() {
                self.focused_role = Some(role);
            }
        }
        match role.or(self.active_role) {
            Some(AgentTerminalRole::Architect) => self.architect_terminal.push_line(line.clone()),
            Some(AgentTerminalRole::Reviewer) => self.reviewer_terminal.push_line(line.clone()),
            Some(AgentTerminalRole::Dev) => self.dev_terminal.push_line(line.clone()),
            Some(AgentTerminalRole::Auditor) => self.auditor_terminal.push_line(line.clone()),
            None => {}
        }
        self.lines.push(line);
        if self.lines.len() > 200 {
            self.lines.drain(..self.lines.len() - 200);
        }
    }

    fn visible_roles(&self) -> Vec<AgentTerminalRole> {
        if !self.architect_terminal.is_empty() || !self.reviewer_terminal.is_empty() {
            let mut roles = vec![AgentTerminalRole::Architect];
            if !self.reviewer_terminal.is_empty() {
                roles.push(AgentTerminalRole::Reviewer);
            }
            roles
        } else {
            vec![AgentTerminalRole::Dev, AgentTerminalRole::Auditor]
        }
    }

    fn focused_role(&self) -> AgentTerminalRole {
        let roles = self.visible_roles();
        self.focused_role
            .filter(|role| roles.contains(role))
            .unwrap_or_else(|| roles[0])
    }

    fn cycle_focus(&mut self) {
        let roles = self.visible_roles();
        let current = self.focused_role();
        let next = roles
            .iter()
            .position(|role| *role == current)
            .map(|idx| roles[(idx + 1) % roles.len()])
            .unwrap_or(roles[0]);
        self.focused_role = Some(next);
    }

    fn toggle_split_panes(&mut self) {
        self.split_panes = !self.split_panes;
    }

    fn terminal_mut(&mut self, role: AgentTerminalRole) -> &mut AgentTerminal {
        match role {
            AgentTerminalRole::Architect => &mut self.architect_terminal,
            AgentTerminalRole::Reviewer => &mut self.reviewer_terminal,
            AgentTerminalRole::Dev => &mut self.dev_terminal,
            AgentTerminalRole::Auditor => &mut self.auditor_terminal,
        }
    }

    fn terminal(&self, role: AgentTerminalRole) -> &AgentTerminal {
        match role {
            AgentTerminalRole::Architect => &self.architect_terminal,
            AgentTerminalRole::Reviewer => &self.reviewer_terminal,
            AgentTerminalRole::Dev => &self.dev_terminal,
            AgentTerminalRole::Auditor => &self.auditor_terminal,
        }
    }

    fn scroll_focused_up(&mut self, amount: usize) {
        let role = self.focused_role();
        self.terminal_mut(role).scroll_up(amount);
    }

    fn scroll_focused_down(&mut self, amount: usize) {
        let role = self.focused_role();
        self.terminal_mut(role).scroll_down(amount);
    }

    fn scroll_focused_top(&mut self) {
        let role = self.focused_role();
        self.terminal_mut(role).scroll_top();
    }

    fn scroll_focused_bottom(&mut self) {
        let role = self.focused_role();
        self.terminal_mut(role).scroll_bottom();
    }

    fn copy_focused(&self) -> std::io::Result<&'static str> {
        let role = self.focused_role();
        let terminal = self.terminal(role);
        copy_to_clipboard_osc52(&terminal.full_text())?;
        Ok(terminal.title())
    }

    fn push_error(&mut self, error: impl Into<String>) {
        self.push_line(format!("ERRO: {}", error.into()));
    }
}

fn copy_to_clipboard_osc52(text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let encoded = base64_encode(text.as_bytes());
    let mut stdout = std::io::stdout();
    write!(stdout, "\x1b]52;c;{}\x07", encoded)?;
    stdout.flush()
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);

        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
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
    Setup,
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
            fields: vec![PromptField {
                label: "Título do plano",
                optional: false,
                default: None,
            }],
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
                PromptField {
                    label: "CLI para a IA Dev (ex: claude, gemini)",
                    optional: false,
                    default: None,
                },
                PromptField {
                    label: "CLI para a Auditora (obrigatório)",
                    optional: false,
                    default: None,
                },
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
            fields: vec![PromptField {
                label: "Notas para a IA (instruções de ajuste)",
                optional: false,
                default: None,
            }],
            current: 0,
            buffer: String::new(),
            collected: Vec::new(),
            next: PromptNext::GateEnrich,
        }
    }

    fn for_setup() -> Self {
        Self {
            title: "Configuração Inicial",
            fields: vec![
                PromptField {
                    label: "CLI para a IA Dev (ex: claude, gemini)",
                    optional: false,
                    default: None,
                },
                PromptField {
                    label: "CLI para a Auditora (obrigatório)",
                    optional: false,
                    default: None,
                },
            ],
            current: 0,
            buffer: String::new(),
            collected: Vec::new(),
            next: PromptNext::Setup,
        }
    }

    fn for_continue(plan_id: String) -> Self {
        Self {
            title: "Continuar Planejamento",
            fields: vec![
                PromptField {
                    label: "CLI para o Arquiteto (ex: claude, gemini)",
                    optional: false,
                    default: None,
                },
                PromptField {
                    label: "CLI para o Revisor do plano (opcional; não implementa código)",
                    optional: true,
                    default: None,
                },
                PromptField {
                    label: "Briefing humano inicial para o Arquiteto",
                    optional: false,
                    default: None,
                },
                PromptField {
                    label: "Máximo de turnos",
                    optional: false,
                    default: Some("10"),
                },
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
    memory_view: MemoryViewState,
    plans: Vec<PlanSummary>,
    summary: ProjectSummary,
    home_memory: String,
    orchestrator_dir: PathBuf,
    exec: ExecState,
    log_rx: Option<crate::core::stream::LogRx>,
    gate_tx: Option<crate::core::stream::GateTx>,
}

impl TuiApp {
    fn new(
        orchestrator_dir: PathBuf,
        summary: ProjectSummary,
        plans: Vec<PlanSummary>,
        home_memory: String,
    ) -> Self {
        let selector = SelectorState::new(plans.len());
        Self {
            view: AppView::Home,
            home: HomeState::new(),
            selector,
            selector_ctx: SelectorCtx::ForDashboard,
            dash_data: DashboardState::default(),
            dash_ui: DashboardUiState::new(0),
            memory_view: MemoryViewState::new(vec![]),
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
    if crate::core::config::Config::load().is_err() {
        crate::commands::init::execute(true, false).await?;
    }
    let config = crate::core::config::Config::load()?;
    let orchestrator_dir = config.orchestrator_dir.clone();

    let summary = Db::get_home_summary(orchestrator_dir.clone())
        .await
        .unwrap_or_default();
    let plans = load_plans(&orchestrator_dir).await;
    let home_memory = load_ai_memory_feed();

    let mut app = TuiApp::new(orchestrator_dir.clone(), summary, plans, home_memory);
    if config.dev_cli.is_none() {
        app.view = AppView::Prompt(PromptState::for_setup());
    }

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
        // Drena o canal a cada tick — não espera keypress para exibir logs
        if matches!(app.view, AppView::Executing | AppView::Gate) {
            if let Some(ref rx) = app.log_rx {
                loop {
                    match rx.try_recv() {
                        Ok(crate::core::stream::LogEvent::Line(s)) => {
                            app.exec.push_line(s);
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
                            app.summary = Db::get_home_summary(app.orchestrator_dir.clone())
                                .await
                                .unwrap_or_default();
                            app.plans = load_plans(&app.orchestrator_dir).await;
                            app.home_memory = load_ai_memory_feed();
                            app.view = AppView::Home;
                            break;
                        }
                        Ok(crate::core::stream::LogEvent::Failed(e)) => {
                            app.exec.push_error(e);
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
                            let has_errors = app.exec.lines.iter().any(|l| {
                                l.starts_with("ERRO:") || l.contains("❌") || l.contains("ERRO")
                            });
                            if has_errors {
                                app.exec.push_line(
                                    "(thread finalizada — leia os erros acima e pressione q)"
                                        .to_string(),
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

        terminal.draw(|f| render_app(f, app))?;

        if !event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match dispatch_key(app, key.code) {
            LoopCmd::Continue => {}
            LoopCmd::Quit => break,
            LoopCmd::GoTo(view) => {
                app.view = view;
            }
            LoopCmd::StartPlanning {
                plan_id,
                cli1,
                cli2,
                initial_notes,
                max_turns,
            } => {
                use crate::core::stream::*;
                let (log_tx, log_rx) = std::sync::mpsc::sync_channel::<LogEvent>(256);
                let (gate_tx, gate_rx) = std::sync::mpsc::sync_channel::<GateDecision>(1);
                let dir = app.orchestrator_dir.clone();
                let has_reviewer = cli2.is_some();
                std::thread::spawn(move || match tokio::runtime::Runtime::new() {
                    Err(e) => {
                        let _ = log_tx.send(LogEvent::Failed(format!("runtime: {}", e)));
                    }
                    Ok(rt) => {
                        let result = rt.block_on(run_planning_background(
                            dir,
                            plan_id,
                            cli1,
                            cli2,
                            initial_notes,
                            max_turns,
                            log_tx.clone(),
                            gate_rx,
                        ));
                        if let Err(e) = result {
                            let _ = log_tx.send(LogEvent::Failed(e.to_string()));
                        }
                    }
                });
                app.exec = ExecState::new("Planejamento em andamento...".to_string());
                app.exec.has_planning_reviewer = has_reviewer;
                app.log_rx = Some(log_rx);
                app.gate_tx = Some(gate_tx);
                app.view = AppView::Executing;
            }
            LoopCmd::StartDev {
                plan_id,
                cli_dev,
                cli_audit,
            } => {
                use crate::core::stream::*;
                let (log_tx, log_rx) = std::sync::mpsc::sync_channel::<LogEvent>(256);
                let (gate_tx, gate_rx) = std::sync::mpsc::sync_channel::<GateDecision>(1);
                std::thread::spawn(move || match tokio::runtime::Runtime::new() {
                    Err(e) => {
                        let _ = log_tx.send(LogEvent::Failed(format!("runtime: {}", e)));
                    }
                    Ok(rt) => {
                        let result = rt.block_on(crate::commands::run::execute_tui(
                            plan_id,
                            cli_dev,
                            cli_audit,
                            log_tx.clone(),
                            gate_rx,
                        ));
                        if let Err(e) = result {
                            let _ = log_tx.send(LogEvent::Failed(e.to_string()));
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
                    let mut exec =
                        ExecState::new("🔍 Deep Scan — Erro de Configuração".to_string());
                    exec.push_line("ERRO: 'ai-memory' não encontrado no PATH.".to_string());
                    exec.push_line("Instale o ai-memory CLI e tente novamente.".to_string());
                    exec.push_line("(pressione q para voltar ao menu)".to_string());
                    app.exec = exec;
                    app.view = AppView::Executing;
                } else {
                    let (log_tx, log_rx) = std::sync::mpsc::sync_channel::<LogEvent>(512);
                    let (gate_tx, gate_rx) = std::sync::mpsc::sync_channel::<GateDecision>(1);
                    let dir = app.orchestrator_dir.clone();
                    // Fix 1: Runtime::new() sem unwrap — erro propagado pelo canal
                    std::thread::spawn(move || match tokio::runtime::Runtime::new() {
                        Err(e) => {
                            let _ = log_tx.send(LogEvent::Failed(format!(
                                "Falha ao criar runtime tokio: {}",
                                e
                            )));
                        }
                        Ok(rt) => {
                            let result = rt.block_on(run_deep_scan(dir, log_tx.clone(), gate_rx));
                            if let Err(e) = result {
                                let _ = log_tx.send(LogEvent::Failed(e.to_string()));
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
                app.summary = Db::get_home_summary(app.orchestrator_dir.clone())
                    .await
                    .unwrap_or_default();
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
            LoopCmd::OpenMemory => {
                let pages = list_ai_memory_pages()
                    .into_iter()
                    .map(|p| (p.path, p.title))
                    .collect();
                app.memory_view = MemoryViewState::new(pages);
                app.view = AppView::Memory;
            }
            LoopCmd::SaveSetup { dev_cli, audit_cli } => {
                let config_path = app.orchestrator_dir.join("config.json");
                if let Ok(raw) = std::fs::read_to_string(&config_path) {
                    if let Ok(mut stored) =
                        serde_json::from_str::<crate::core::config::StoredConfig>(&raw)
                    {
                        stored.dev_cli = Some(dev_cli);
                        stored.audit_cli = audit_cli;
                        let _ = crate::commands::write_json(&config_path, &stored);
                    }
                }
                app.view = AppView::Home;
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
    StartPlanning {
        plan_id: String,
        cli1: String,
        cli2: Option<String>,
        initial_notes: String,
        max_turns: usize,
    },
    StartDev {
        plan_id: String,
        cli_dev: String,
        cli_audit: Option<String>,
    },
    StartScan,
    CreatePlan {
        title: String,
    },
    GateDecide(crate::core::stream::GateDecision),
    LoadDashboard(String),
    ReloadDashboard,
    LoadMemory(String),
    SaveSetup {
        dev_cli: String,
        audit_cli: Option<String>,
    },
    OpenMemory,
}

fn dispatch_key(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match &app.view {
        AppView::Home => dispatch_home(app, key),
        AppView::Selector(_) => dispatch_selector(app, key),
        AppView::Dashboard => dispatch_dashboard(app, key),
        AppView::Memory => dispatch_memory(app, key),
        AppView::Prompt(_) => dispatch_prompt(app, key),
        AppView::Executing => dispatch_executing(app, key),
        AppView::Gate => dispatch_gate(app, key),
    }
}

fn dispatch_memory(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    use memory::MemoryFocus;
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::GoTo(AppView::Home),
        KeyCode::Tab => {
            app.memory_view.toggle_focus();
            LoopCmd::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            match app.memory_view.focus {
                MemoryFocus::List => app.memory_view.move_up(),
                MemoryFocus::Content => app.memory_view.scroll_up(),
            }
            LoopCmd::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            match app.memory_view.focus {
                MemoryFocus::List => app.memory_view.move_down(),
                MemoryFocus::Content => app.memory_view.scroll_down(),
            }
            LoopCmd::Continue
        }
        KeyCode::Enter => {
            match app.memory_view.focus {
                MemoryFocus::List => app.memory_view.load_preview(),
                MemoryFocus::Content => {}
            }
            LoopCmd::Continue
        }
        KeyCode::Char('g') => {
            app.memory_view.scroll_top();
            LoopCmd::Continue
        }
        KeyCode::Char('s') if app.memory_view.focus == MemoryFocus::List => LoopCmd::StartScan,
        KeyCode::Char('r') if app.memory_view.focus == MemoryFocus::List => LoopCmd::OpenMemory,
        _ => LoopCmd::Continue,
    }
}

fn dispatch_home(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::Quit,
        KeyCode::Char('r') => {
            app.home_memory = load_ai_memory_feed();
            LoopCmd::Continue
        }
        KeyCode::Up => {
            app.home.move_up();
            LoopCmd::Continue
        }
        KeyCode::Down => {
            app.home.move_down();
            LoopCmd::Continue
        }
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
                4 => LoopCmd::OpenMemory,
                _ => LoopCmd::Quit,
            }
        }
        _ => LoopCmd::Continue,
    }
}

fn dispatch_selector(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::GoTo(AppView::Home),
        KeyCode::Up => {
            app.selector.move_up();
            LoopCmd::Continue
        }
        KeyCode::Down => {
            app.selector.move_down(app.plans.len());
            LoopCmd::Continue
        }
        KeyCode::Enter => {
            let Some(plan) = app.selector.pick(&app.plans) else {
                return LoopCmd::Continue;
            };
            let plan_id = plan.id.clone();
            let ctx = app.selector_ctx.clone();
            match ctx {
                SelectorCtx::ForDashboard => LoopCmd::LoadDashboard(plan_id),
                SelectorCtx::ForContinue => {
                    LoopCmd::GoTo(AppView::Prompt(PromptState::for_continue(plan_id)))
                }
                SelectorCtx::ForDev => {
                    LoopCmd::GoTo(AppView::Prompt(PromptState::for_dev(plan_id)))
                }
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
    let AppView::Prompt(ref mut ps) = app.view else {
        return LoopCmd::Continue;
    };
    match key {
        KeyCode::Esc => LoopCmd::GoTo(AppView::Home),
        KeyCode::Backspace => {
            ps.buffer.pop();
            LoopCmd::Continue
        }
        KeyCode::Char(c) => {
            ps.buffer.push(c);
            LoopCmd::Continue
        }
        KeyCode::Enter => {
            // Aplica default se buffer vazio e campo tem default
            if ps.buffer.is_empty() {
                if let Some(def) = ps.fields[ps.current].default {
                    ps.buffer = def.to_string();
                }
            }
            if ps.buffer.trim().is_empty() && !ps.fields[ps.current].optional {
                return LoopCmd::Continue;
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
                    let cli2 = if collected[1].is_empty() {
                        None
                    } else {
                        Some(collected[1].clone())
                    };
                    let initial_notes = collected[2].clone();
                    let max_turns: usize = collected[3].parse().unwrap_or(10);
                    LoopCmd::StartPlanning {
                        plan_id,
                        cli1,
                        cli2,
                        initial_notes,
                        max_turns,
                    }
                }
                PromptNext::NewPlan => LoopCmd::CreatePlan {
                    title: collected[0].clone(),
                },
                PromptNext::GateEnrich => {
                    use crate::core::stream::GateDecision;
                    LoopCmd::GateDecide(GateDecision::Enrich(collected[0].clone()))
                }
                PromptNext::DevMode { plan_id } => {
                    let cli_dev = collected[0].clone();
                    let cli_audit = Some(collected[1].clone());
                    LoopCmd::StartDev {
                        plan_id: plan_id.clone(),
                        cli_dev,
                        cli_audit,
                    }
                }
                PromptNext::Setup => {
                    let dev_cli = collected[0].clone();
                    let audit_cli = Some(collected[1].clone());
                    LoopCmd::SaveSetup { dev_cli, audit_cli }
                }
            }
        }
        _ => LoopCmd::Continue,
    }
}

fn dispatch_executing(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => LoopCmd::GoTo(AppView::Home),
        KeyCode::Tab => {
            app.exec.cycle_focus();
            LoopCmd::Continue
        }
        KeyCode::Char('v') => {
            app.exec.toggle_split_panes();
            LoopCmd::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.exec.scroll_focused_up(1);
            LoopCmd::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.exec.scroll_focused_down(1);
            LoopCmd::Continue
        }
        KeyCode::PageUp => {
            app.exec.scroll_focused_up(10);
            LoopCmd::Continue
        }
        KeyCode::PageDown => {
            app.exec.scroll_focused_down(10);
            LoopCmd::Continue
        }
        KeyCode::Char('g') => {
            app.exec.scroll_focused_top();
            LoopCmd::Continue
        }
        KeyCode::Char('G') => {
            app.exec.scroll_focused_bottom();
            LoopCmd::Continue
        }
        KeyCode::Char('y') => {
            match app.exec.copy_focused() {
                Ok(title) => app
                    .exec
                    .push_line(format!("📋 Painel '{}' copiado.", title)),
                Err(e) => app
                    .exec
                    .push_error(format!("falha ao copiar painel: {}", e)),
            }
            LoopCmd::Continue
        }
        _ => LoopCmd::Continue,
    }
}

fn dispatch_gate(app: &mut TuiApp, key: KeyCode) -> LoopCmd {
    use crate::core::stream::GateDecision;
    match key {
        KeyCode::Tab => {
            app.exec.cycle_focus();
            return LoopCmd::Continue;
        }
        KeyCode::Char('v') => {
            app.exec.toggle_split_panes();
            return LoopCmd::Continue;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.exec.scroll_focused_up(1);
            return LoopCmd::Continue;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.exec.scroll_focused_down(1);
            return LoopCmd::Continue;
        }
        KeyCode::PageUp => {
            app.exec.scroll_focused_up(10);
            return LoopCmd::Continue;
        }
        KeyCode::PageDown => {
            app.exec.scroll_focused_down(10);
            return LoopCmd::Continue;
        }
        KeyCode::Char('g') => {
            app.exec.scroll_focused_top();
            return LoopCmd::Continue;
        }
        KeyCode::Char('G') => {
            app.exec.scroll_focused_bottom();
            return LoopCmd::Continue;
        }
        KeyCode::Char('y') => {
            match app.exec.copy_focused() {
                Ok(title) => app
                    .exec
                    .push_line(format!("📋 Painel '{}' copiado.", title)),
                Err(e) => app
                    .exec
                    .push_error(format!("falha ao copiar painel: {}", e)),
            }
            return LoopCmd::Continue;
        }
        _ => {}
    }
    if app.exec.gate_type == "planning" {
        match key {
            KeyCode::Enter | KeyCode::Char('e') => {
                LoopCmd::GoTo(AppView::Prompt(PromptState::for_enrich()))
            }
            KeyCode::Char('c') => LoopCmd::GateDecide(GateDecision::Finalize),
            KeyCode::Char('r') if app.exec.has_planning_reviewer => {
                LoopCmd::GateDecide(GateDecision::Review)
            }
            KeyCode::Char('f') => LoopCmd::GateDecide(GateDecision::Finalize),
            KeyCode::Char('a') => LoopCmd::GateDecide(GateDecision::Abort),
            KeyCode::Esc => LoopCmd::Continue,
            _ => LoopCmd::Continue,
        }
    } else {
        match key {
            KeyCode::Char('c') | KeyCode::Enter => LoopCmd::GateDecide(GateDecision::Continue),
            KeyCode::Char('e') => LoopCmd::GoTo(AppView::Prompt(PromptState::for_enrich())),
            KeyCode::Char('f') => LoopCmd::GateDecide(GateDecision::Finalize),
            KeyCode::Char('a') | KeyCode::Esc => LoopCmd::GateDecide(GateDecision::Abort),
            _ => LoopCmd::Continue,
        }
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
        AppView::Memory => memory::render(f, &mut app.memory_view, chunks[1]),
        AppView::Prompt(ps) => render_prompt(f, ps, chunks[1]),
        AppView::Executing => render_executing(f, &app.exec, chunks[1]),
        AppView::Gate => render_gate(f, &app.exec, chunks[1]),
    }

    render_footer(f, app, chunks[2]);
}

fn render_banner(f: &mut Frame, area: Rect) {
    let memory_project = crate::core::memory::ai_memory_project_label();
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  AI ORCHESTRATOR",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
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
            Span::styled("│  v0.1.0  ", Style::default().fg(Color::Gray)),
            Span::styled("│  Memória: ", Style::default().fg(Color::Gray)),
            Span::styled(
                memory_project,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    let banner = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " AI Orchestrator ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
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
        let display = if val.is_empty() {
            "(padrão)".to_string()
        } else {
            val.clone()
        };
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
        let def_hint = field
            .default
            .map(|d| format!(" [padrão: {}]", d))
            .unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!("  ▶ {}{}{}", field.label, suffix, def_hint),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
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
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        )
        .wrap(Wrap { trim: false });
    let _ = text; // suprime warning
    f.render_widget(panel, area);
}

fn render_executing(f: &mut Frame, exec: &ExecState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(5)])
        .split(area);

    let visible: Vec<&str> = exec
        .lines
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|s| s.as_str())
        .collect();
    let text = visible.join("\n");
    let panel = Paragraph::new(text.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    format!(" ⠋ {} ", exec.title),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(panel, chunks[0]);
    render_agent_panes(f, exec, chunks[1]);
}

fn render_gate(f: &mut Frame, exec: &ExecState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(area);

    let preview = exec.gate_content.as_deref().unwrap_or("");
    let preview_text: String = preview.lines().take(2).collect::<Vec<_>>().join("\n");
    let gate_options = match exec.gate_type.as_str() {
        "planning" if exec.has_planning_reviewer => "\n  [E] / Enter  →  Enriquecer antes de outro turno\n  [R]          →  Enviar ao Revisor\n  [C] / [F]    →  Congelar/finalizar planejamento\n  [A]          →  Abortar",
        "planning" => "\n  [E] / Enter  →  Enriquecer antes de outro turno\n  [C] / [F]    →  Congelar/finalizar planejamento\n  [A]          →  Abortar",
        "diff_review" => "\n  [C] / Enter  →  Aprovar diff\n  [E]          →  Enriquecer (notas para o Dev)\n  [A] / Esc    →  Rejeitar diff",
        "apply" => "\n  [C] / Enter  →  Aplicar patch ao workspace\n  [A] / Esc    →  Pular (não aplicar)",
        "inter_task" => "\n  [C] / Enter  →  Próxima tarefa\n  [E]          →  Repetir com notas\n  [A] / Esc    →  Encerrar Dev Mode",
        "error" => "\n  [C] / Enter  →  Continuar mesmo assim (cuidado)\n  [A] / Esc    →  Abortar tarefa",
        _ => "\n  [C] / Enter  →  Continuar\n  [A] / Esc    →  Abortar",
    };
    let gate_text = if preview_text.trim().is_empty() {
        gate_options.to_string()
    } else {
        format!("{}\n{}", preview_text, gate_options)
    };
    let gate = Paragraph::new(gate_text)
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    " ✋ Portão — Decisão Necessária ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        );
    f.render_widget(gate, chunks[0]);
    render_agent_panes(f, exec, chunks[1]);
}

fn render_agent_panes(f: &mut Frame, exec: &ExecState, area: Rect) {
    let focused = exec.focused_role();
    let roles = exec.visible_roles();

    if !exec.split_panes {
        exec.terminal(focused).render(f, area, true);
        return;
    }

    let constraints = vec![Constraint::Percentage(100 / roles.len() as u16); roles.len()];
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (idx, role) in roles.iter().copied().enumerate() {
        exec.terminal(role).render(f, panes[idx], focused == role);
    }
}

fn render_footer(f: &mut Frame, app: &TuiApp, area: Rect) {
    let text = match &app.view {
        AppView::Home => {
            "  ↑↓: Navegar   Enter: Executar   1-6: Atalho   r: Recarregar Memória   q: Sair"
        }
        AppView::Selector(_) => "  ↑↓: Navegar   Enter: Selecionar   q/Esc: Voltar ao Menu",
        AppView::Dashboard => {
            "  ↑↓: Feed   Enter: Carregar Handoff   r: Recarregar   q/Esc: Voltar"
        }
        AppView::Prompt(_) => "  Enter: Confirmar   Backspace: Apagar   Esc: Cancelar",
        AppView::Memory => {
            "  ↑↓/jk: Navegar   Enter: Ler página   s: Sincronizar   r: Refresh   q/Esc: Voltar"
        }
        AppView::Executing => {
            "  Tab: painel   v: lado a lado/pilha   ↑↓/jk/PgUp/PgDn: scroll   y: copiar   q: voltar"
        }
        AppView::Gate if app.exec.gate_type == "planning" && app.exec.has_planning_reviewer => {
            "  Enter/E: Enriquecer   R: Revisor   C/F: Congelar   Tab/v/↑↓: painel   y: copiar"
        }
        AppView::Gate if app.exec.gate_type == "planning" => {
            "  Enter/E: Enriquecer   C/F: Congelar   Tab/v/↑↓: painel   y: copiar"
        }
        AppView::Gate => {
            "  C/Enter: Continuar   E: Enriquecer   Tab/v/↑↓: painel   y: copiar   A/Esc: Abortar"
        }
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

async fn load_plans(orchestrator_dir: &Path) -> Vec<PlanSummary> {
    let Ok(db) = Db::open_readonly(orchestrator_dir).await else {
        return vec![];
    };
    db.list_plans().await.unwrap_or_default()
}

/// Carrega as notas e páginas recentes do ai-memory via CLI.
fn load_ai_memory_feed() -> String {
    for args in [
        vec![
            "search",
            "fase OR tui OR notes OR implementacao",
            "--limit",
            "8",
        ],
        vec!["search", "ai-orchestrator", "--limit", "5"],
        vec!["search", "tui", "--limit", "5"],
    ] {
        if let Ok(output) = std::process::Command::new("ai-memory").args(&args).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if !text.trim().is_empty() {
                    let clean = text.replace("<mark>", "").replace("</mark>", "");
                    let trimmed: String = clean.lines().take(50).collect::<Vec<_>>().join("\n");
                    return trimmed;
                }
            }
        }
    }
    "── Feed de Conhecimento ──\n\nai-memory não disponível\nou sem páginas recentes.\n\nPressione 'r' para tentar novamente.".to_string()
}

/// Cria um novo plano diretamente via DB, sem sair do raw mode.
async fn create_plan_native(orchestrator_dir: &Path, title: String) -> Result<()> {
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
        else {
            continue;
        };

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
                            pages.push(MemoryPageInfo {
                                path: p.clone(),
                                title,
                            });
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
            .map(|r| {
                r.split('/')
                    .next()
                    .unwrap_or(r)
                    .strip_suffix(".md")
                    .unwrap_or(r)
                    .to_string()
            })
            .unwrap_or_else(|| {
                format!(
                    "scan-{:016x}",
                    path.bytes()
                        .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))
                )
            })
    };

    if path.starts_with("handoffs/") {
        let seg = path.strip_prefix("handoffs/").unwrap_or(path);
        ("handoff_created", run_id_from_path(seg))
    } else if path.starts_with("decisions/") {
        (
            "gate_passed",
            format!(
                "scan-decisions-{:016x}",
                path.bytes()
                    .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))
            ),
        )
    } else if path.starts_with("plans/") || path.starts_with("notes/") {
        (
            "gate_enriched",
            format!(
                "scan-notes-{:016x}",
                path.bytes()
                    .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))
            ),
        )
    } else {
        (
            "handoff_created",
            format!(
                "scan-other-{:016x}",
                path.bytes()
                    .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))
            ),
        )
    }
}

async fn run_deep_scan(
    orchestrator_dir: PathBuf,
    log_tx: crate::core::stream::LogTx,
    _gate_rx: crate::core::stream::GateRx,
) -> anyhow::Result<()> {
    use crate::core::db::EventType;
    use crate::core::stream::LogEvent;

    let send = |msg: String| {
        let _ = log_tx.send(LogEvent::Line(msg));
    };

    send("🔍 Listando páginas do ai-memory...".to_string());
    let pages = list_ai_memory_pages();

    if pages.is_empty() {
        send(
            "⚠ Nenhuma página encontrada. Verifique se o ai-memory CLI está disponível."
                .to_string(),
        );
        let _ = log_tx.send(LogEvent::Done);
        return Ok(());
    }
    send(format!(
        "📄 {} páginas encontradas. Iniciando ingestão...",
        pages.len()
    ));

    let config = crate::core::config::Config::load()?;
    let db = crate::core::db::Db::open(
        &orchestrator_dir,
        &config.workspace_dir,
        &format!("deep-scan-{}", chrono::Utc::now().timestamp()),
        "deep-scan",
        "scan",
        "HEAD",
        "",
        "",
    )?;

    let mut ingested = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for page in &pages {
        let (event_type_str, run_id) = extract_metadata_from_path(&page.path);
        let event_type = match event_type_str {
            "handoff_created" => EventType::HandoffCreated,
            "gate_passed" => EventType::GatePassed,
            _ => EventType::GateEnriched,
        };

        match db
            .reconstruct_event(&run_id, event_type, &page.title, &page.path)
            .await
        {
            Ok(true) => {
                ingested += 1;
                send(format!("  ✅ {}", page.path));
            }
            Ok(false) => {
                skipped += 1;
            }
            Err(e) => {
                failed += 1;
                send(format!("  ❌ {}: {}", page.path, e));
            }
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
#[allow(clippy::too_many_arguments)]
async fn run_planning_background(
    orchestrator_dir: PathBuf,
    plan_id: String,
    cli1: String,
    cli2: Option<String>,
    initial_notes: String,
    max_turns: usize,
    log_tx: crate::core::stream::LogTx,
    gate_rx: crate::core::stream::GateRx,
) -> anyhow::Result<()> {
    use crate::commands::plan::run_planning_loop;
    use crate::core::db::Db;
    use uuid::Uuid;

    let config = crate::core::config::Config::load()?;

    let agents_owned: Vec<(String, String, String)> = if let Some(ref cli_reviewer) = cli2 {
        vec![
            (
                "architect".to_string(),
                "Arquiteto".to_string(),
                cli1.clone(),
            ),
            (
                "reviewer".to_string(),
                "Revisor de Planejamento".to_string(),
                cli_reviewer.clone(),
            ),
        ]
    } else {
        vec![(
            "architect".to_string(),
            "Arquiteto".to_string(),
            cli1.clone(),
        )]
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

    run_planning_loop(
        &db,
        &orchestrator_dir,
        &plan_id,
        &agents,
        max_turns,
        Some(&initial_notes),
        Some(log_tx.clone()),
        Some(gate_rx),
    )
    .await?;

    let _ = log_tx.send(crate::core::stream::LogEvent::Done);
    Ok(())
}
