use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

#[derive(PartialEq, Clone)]
pub enum MemoryFocus {
    List,
    Content,
}

pub struct MemoryViewState {
    pub pages: Vec<(String, String)>, // (path, title)
    pub list_state: ListState,
    pub preview: String,
    pub preview_scroll: u16,
    pub page_count: usize,
    pub focus: MemoryFocus,
}

impl MemoryViewState {
    pub fn new(pages: Vec<(String, String)>) -> Self {
        let count = pages.len();
        let mut list_state = ListState::default();
        if !pages.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            pages,
            list_state,
            preview: "Selecione uma página e pressione Enter para ler.\n\nUse Tab para alternar o foco para este painel e navegar com ↑↓.".to_string(),
            preview_scroll: 0,
            page_count: count,
            focus: MemoryFocus::List,
        }
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            MemoryFocus::List => MemoryFocus::Content,
            MemoryFocus::Content => MemoryFocus::List,
        };
    }

    pub fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    pub fn move_up(&mut self) {
        let i = self.selected();
        if i > 0 {
            self.list_state.select(Some(i - 1));
        }
    }

    pub fn move_down(&mut self) {
        let i = self.selected();
        if i + 1 < self.pages.len() {
            self.list_state.select(Some(i + 1));
        }
    }

    pub fn scroll_up(&mut self) {
        self.preview_scroll = self.preview_scroll.saturating_sub(3);
    }

    pub fn scroll_down(&mut self) {
        self.preview_scroll = self.preview_scroll.saturating_add(3);
    }

    pub fn scroll_top(&mut self) {
        self.preview_scroll = 0;
    }

    pub fn selected_path(&self) -> Option<&str> {
        self.pages.get(self.selected()).map(|(p, _)| p.as_str())
    }

    pub fn load_preview(&mut self) {
        let Some(path) = self.selected_path() else {
            return;
        };
        let path = path.to_string();
        match std::process::Command::new("ai-memory")
            .args(["read-page", "--path", &path])
            .output()
        {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                self.preview = if text.trim().is_empty() {
                    "(página vazia)".to_string()
                } else {
                    text
                };
                self.preview_scroll = 0;
                self.focus = MemoryFocus::Content;
            }
            _ => {
                self.preview = format!("Erro ao carregar: {}", path);
            }
        }
    }
}

pub fn render(f: &mut Frame, state: &mut MemoryViewState, area: Rect) {
    let memory_project = crate::core::memory::ai_memory_project_label();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(chunks[0]);

    let list_focused = state.focus == MemoryFocus::List;
    let content_focused = state.focus == MemoryFocus::Content;

    // ── Lista de páginas ──────────────────────────────────────────────────────
    let items: Vec<ListItem> = state
        .pages
        .iter()
        .map(|(path, title)| {
            let short_path = path
                .strip_prefix("notes/")
                .or_else(|| path.strip_prefix("sessions/"))
                .or_else(|| path.strip_prefix("decisions/"))
                .or_else(|| path.strip_prefix("handoffs/"))
                .unwrap_or(path);
            ListItem::new(vec![
                Line::from(Span::styled(
                    format!(" {}", title),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(
                    format!("   {}", short_path),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect();

    let list_border_color = if list_focused {
        Color::Yellow
    } else {
        Color::Cyan
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(list_border_color))
                .title(Span::styled(
                    format!(
                        " AI Memory: {} ({} páginas) ",
                        memory_project, state.page_count
                    ),
                    Style::default()
                        .fg(list_border_color)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, main[0], &mut state.list_state);

    // ── Preview da página ─────────────────────────────────────────────────────
    let content_border_color = if content_focused {
        Color::Yellow
    } else {
        Color::Blue
    };
    let total_lines = state.preview.lines().count() as u16;
    let visible_height = main[1].height.saturating_sub(2);
    let scroll_info = if total_lines > visible_height {
        format!(
            " Conteúdo  [{}/{}] ",
            state.preview_scroll + 1,
            total_lines.saturating_sub(visible_height) + 1
        )
    } else {
        " Conteúdo ".to_string()
    };

    let preview = Paragraph::new(state.preview.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(content_border_color))
                .title(Span::styled(
                    scroll_info,
                    Style::default()
                        .fg(content_border_color)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false })
        .scroll((state.preview_scroll, 0));

    f.render_widget(preview, main[1]);

    // ── Footer contextual ─────────────────────────────────────────────────────
    let footer_text = if list_focused {
        " [Lista]  ↑↓/jk: navegar   Enter: ler   Tab: foco→conteúdo   s: sync   r: refresh   q: voltar"
    } else {
        " [Conteúdo]  ↑↓/jk: scroll   Tab: foco→lista   q: voltar"
    };

    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );

    f.render_widget(footer, chunks[1]);
}
