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

/// Ponto de entrada do Dashboard em Ratatui (Fase 4 - Read Only).
pub fn run_dashboard(plan_id: Option<String>) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, plan_id);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, _plan_id: Option<String>) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                    return Ok(());
                }
            }
        }
    }
}

fn ui(f: &mut Frame) {
    let size = f.size();

    // Layout principal: Vertical (Header | Body | Footer)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(5),    // Body
            Constraint::Length(3), // Footer
        ])
        .split(size);

    // === HEADER ===
    let header = Paragraph::new("AI Orchestrator — Plan Dashboard (Read-Only)")
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::ALL).title("Header"));
    f.render_widget(header, chunks[0]);

    // === BODY: Horizontal (Left Tasks | Right Audit) ===
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Left: Lista de Tarefas (mock)
    let tasks: Vec<ListItem> = vec![
        ListItem::new("✅ 1. Criar estrutura de pastas"),
        ListItem::new("⏳ 2. Implementar algoritmo iterativo"),
        ListItem::new("⏳ 3. Adicionar memoização"),
        ListItem::new("[ ] 4. Escrever testes"),
        ListItem::new("[ ] 5. Documentar uso"),
    ];

    let tasks_list = List::new(tasks)
        .block(Block::default().borders(Borders::ALL).title("Tarefas do Plano"));
    f.render_widget(tasks_list, body_chunks[0]);

    // Right: Feed de Auditoria (mock)
    let audit_events: Vec<ListItem> = vec![
        ListItem::new("[2026-05-30 14:51] architect → plano: Análise inicial"),
        ListItem::new("[2026-05-30 14:52] human: Aprovado"),
        ListItem::new("[2026-05-30 14:53] dev → core.py: Implementação iterativa"),
        ListItem::new("[2026-05-30 14:54] auditor: Aprovado (score 92)"),
        ListItem::new("[2026-05-30 14:55] human: Aplicado"),
    ];

    let audit_feed = List::new(audit_events)
        .block(Block::default().borders(Borders::ALL).title("Histórico de Auditoria"));
    f.render_widget(audit_feed, body_chunks[1]);

    // === FOOTER ===
    let footer = Paragraph::new("q: Sair  |  ↑↓: Scroll  |  Fase 4 - Read Only")
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}