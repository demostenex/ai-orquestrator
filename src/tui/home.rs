use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::core::db::ProjectSummary;

// ── Menu ──────────────────────────────────────────────────────────────────────

pub const MENU_LEN: usize = 6;

const MENU_ITEMS: &[(&str, &str)] = &[
    ("1. Novo Plano", "Cria um novo plano com o Arquiteto"),
    ("2. Continuar Planejamento", "Retoma uma sessão de planejamento em andamento"),
    ("3. Executar Modo Dev", "Executa as tarefas de um plano aprovado"),
    ("4. Abrir Dashboard", "Visualiza progresso e auditoria em tempo real"),
    ("5. Sincronizar Histórico", "Ingere páginas do ai-memory no banco de auditoria"),
    ("6. Sair", "Encerra o AI Orchestrator"),
];

// ── State ─────────────────────────────────────────────────────────────────────

pub struct HomeState {
    pub menu_state: ListState,
}

impl HomeState {
    pub fn new() -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        Self { menu_state }
    }

    pub fn selected(&self) -> usize {
        self.menu_state.selected().unwrap_or(0)
    }

    pub fn move_up(&mut self) {
        let i = self.selected();
        if i > 0 {
            self.menu_state.select(Some(i - 1));
        }
    }

    pub fn move_down(&mut self) {
        let i = self.selected();
        if i + 1 < MENU_LEN {
            self.menu_state.select(Some(i + 1));
        }
    }
}

// ── Render (puro — sem loop, sem raw mode) ────────────────────────────────────

pub fn render(f: &mut Frame, state: &mut HomeState, summary: &ProjectSummary, memory: &str, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25), // Menu
            Constraint::Percentage(45), // Status do Projeto
            Constraint::Percentage(30), // AI-Memory Feed
        ])
        .split(area);

    render_menu(f, state, columns[0]);
    render_status(f, state, summary, columns[1]);
    render_memory(f, memory, columns[2]);
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

fn render_status(f: &mut Frame, state: &mut HomeState, summary: &ProjectSummary, area: Rect) {
    let selected_desc = MENU_ITEMS
        .get(state.selected())
        .map(|(_, d)| *d)
        .unwrap_or("");

    let progress = if summary.tasks_total > 0 {
        let pct = (summary.tasks_completed * 100) / summary.tasks_total;
        let filled = (summary.tasks_completed * 20) / summary.tasks_total;
        let bar: String = (0..20usize).map(|i| if i < filled { '█' } else { '░' }).collect();
        format!("[{}] {}%  ({}/{})", bar, pct, summary.tasks_completed, summary.tasks_total)
    } else {
        "Sem tarefas registradas".to_string()
    };

    let last_plan = if summary.last_plan_title.is_empty() {
        "Nenhum plano criado ainda".to_string()
    } else {
        format!("{} ({})", summary.last_plan_title, summary.last_plan_status)
    };

    let last_run = if summary.last_run_at.is_empty() {
        "Nenhum run registrado".to_string()
    } else {
        summary.last_run_at[..19.min(summary.last_run_at.len())].to_string()
    };

    let text = format!(
        "{}\n\n── Projeto ─────────────────────────────\n\n  Último Plano    {}\n  Total de Planos  {}\n\n── Tarefas ──────────────────────────────\n\n  {}\n\n── Último Run ───────────────────────────\n\n  {}",
        selected_desc,
        last_plan,
        summary.plans_count,
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

fn render_memory(f: &mut Frame, memory: &str, area: Rect) {
    let display = if memory.trim().is_empty() {
        "Nenhuma nota carregada.\n\nPressione 'r' na Home para recarregar.".to_string()
    } else {
        memory.to_string()
    };

    let panel = Paragraph::new(display.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray))
                .title(Span::styled(
                    " AI-Memory ",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ))
                .title_alignment(Alignment::Center),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(panel, area);
}
