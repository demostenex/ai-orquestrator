use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::io;

use crate::schemas::PlanSummary;

/// Abre o seletor de planos em Ratatui.
/// `context` é uma frase curta exibida no header (ex: "Abrir no Dashboard").
/// Retorna Some(plan_id) se o usuário selecionou, None se cancelou (q/Esc).
pub fn run(plans: Vec<PlanSummary>, context: &str) -> Result<Option<String>> {
    let mut list_state = ListState::default();
    if !plans.is_empty() {
        list_state.select(Some(0));
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut list_state, &plans, context)?;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(result)
}

fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    list_state: &mut ListState,
    plans: &[PlanSummary],
    context: &str,
) -> Result<Option<String>> {
    loop {
        terminal.draw(|f| render(f, list_state, plans, context))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Up => {
                        let i = list_state.selected().unwrap_or(0);
                        if i > 0 {
                            list_state.select(Some(i - 1));
                        }
                    }
                    KeyCode::Down => {
                        let i = list_state.selected().unwrap_or(0);
                        if i + 1 < plans.len() {
                            list_state.select(Some(i + 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(i) = list_state.selected() {
                            if let Some(plan) = plans.get(i) {
                                return Ok(Some(plan.id.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn render(f: &mut Frame, list_state: &mut ListState, plans: &[PlanSummary], context: &str) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Header
            Constraint::Min(5),    // Lista de planos
            Constraint::Length(3), // Footer
        ])
        .split(area);

    render_header(f, chunks[0], context, plans.len());
    render_list(f, chunks[1], list_state, plans);
    render_footer(f, chunks[2], plans.is_empty());
}

fn render_header(f: &mut Frame, area: Rect, context: &str, count: usize) {
    let text = format!("\n  {}\n  {} plano(s) disponível(is)", context, count);
    let header = Paragraph::new(text)
        .style(Style::default().fg(Color::Cyan))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Selecionar Plano ",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        );
    f.render_widget(header, area);
}

fn render_list(f: &mut Frame, area: Rect, list_state: &mut ListState, plans: &[PlanSummary]) {
    if plans.is_empty() {
        let msg = Paragraph::new(
            "\n\n  Nenhum plano encontrado.\n\n  Volte ao menu e crie um com  '1. Novo Plano'.",
        )
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        f.render_widget(msg, area);
        return;
    }

    let selected_id = list_state
        .selected()
        .and_then(|i| plans.get(i))
        .map(|p| format!(" {} ", p.id))
        .unwrap_or_default();

    let items: Vec<ListItem> = plans
        .iter()
        .map(|p| ListItem::new(format!("  {}", p.title)))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray))
                .title(Span::styled(selected_id, Style::default().fg(Color::Gray)))
                .title_alignment(Alignment::Right),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(25, 25, 60)),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, list_state);
}

fn render_footer(f: &mut Frame, area: Rect, empty: bool) {
    let text = if empty {
        "  q: Voltar ao Menu"
    } else {
        "  ↑↓: Navegar   Enter: Selecionar   q: Voltar"
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
