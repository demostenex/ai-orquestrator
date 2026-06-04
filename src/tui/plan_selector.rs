use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::schemas::PlanSummary;

// ── State ─────────────────────────────────────────────────────────────────────

pub struct SelectorState {
    pub list_state: ListState,
}

impl SelectorState {
    pub fn new(plan_count: usize) -> Self {
        let mut list_state = ListState::default();
        if plan_count > 0 {
            list_state.select(Some(0));
        }
        Self { list_state }
    }

    pub fn selected(&self) -> Option<usize> {
        self.list_state.selected()
    }

    pub fn move_up(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        if i > 0 {
            self.list_state.select(Some(i - 1));
        }
    }

    pub fn move_down(&mut self, plan_count: usize) {
        let i = self.list_state.selected().unwrap_or(0);
        if i + 1 < plan_count {
            self.list_state.select(Some(i + 1));
        }
    }

    pub fn pick<'a>(&self, plans: &'a [PlanSummary]) -> Option<&'a PlanSummary> {
        self.selected().and_then(|i| plans.get(i))
    }
}

// ── Render (puro — sem loop, sem raw mode) ────────────────────────────────────

pub fn render(
    f: &mut Frame,
    state: &mut SelectorState,
    plans: &[PlanSummary],
    context: &str,
    area: Rect,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Contexto
            Constraint::Min(3),    // Lista
        ])
        .split(area);

    // Header de contexto
    let header_text = format!("\n  {}\n  {} plano(s) disponível(is)", context, plans.len());
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Selecionar Plano ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        );
    f.render_widget(header, chunks[0]);

    // Lista de planos ou estado vazio
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
        f.render_widget(msg, chunks[1]);
    } else {
        let selected_id = state
            .pick(plans)
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

        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }
}
