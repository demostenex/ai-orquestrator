use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTerminalRole {
    Architect,
    Reviewer,
    Dev,
    Auditor,
}

impl AgentTerminalRole {
    /// Mapeia a origem estruturada do core (`LineOrigin`) para o papel do pane.
    pub fn from_origin(origin: crate::core::stream::LineOrigin) -> Self {
        use crate::core::stream::LineOrigin;
        match origin {
            LineOrigin::Architect => Self::Architect,
            LineOrigin::Reviewer => Self::Reviewer,
            LineOrigin::Dev => Self::Dev,
            LineOrigin::Auditor => Self::Auditor,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentTerminal {
    title: &'static str,
    lines: Vec<String>,
    max_lines: usize,
    border_color: Color,
    scroll_offset: usize,
}

impl AgentTerminal {
    pub fn new(title: &'static str, border_color: Color) -> Self {
        Self {
            title,
            lines: Vec::new(),
            max_lines: 250,
            border_color,
            scroll_offset: 0,
        }
    }

    pub fn push_line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
        if self.lines.len() > self.max_lines {
            self.lines.drain(..self.lines.len() - self.max_lines);
        }
        self.clamp_scroll();
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn title(&self) -> &'static str {
        self.title
    }

    pub fn full_text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.clamp_scroll();
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_top(&mut self) {
        self.scroll_offset = self.lines.len().saturating_sub(1);
    }

    pub fn scroll_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    fn clamp_scroll(&mut self) {
        self.scroll_offset = self.scroll_offset.min(self.lines.len().saturating_sub(1));
    }

    pub fn render(&self, f: &mut Frame, area: Rect, focused: bool) {
        let visible_lines = area.height.saturating_sub(2).max(1) as usize;
        let text = if self.lines.is_empty() {
            "Aguardando saída do agente.".to_string()
        } else {
            self.full_text()
        };
        let total_lines = text.lines().count();
        let top_line = total_lines
            .saturating_sub(visible_lines.saturating_add(self.scroll_offset))
            .min(u16::MAX as usize) as u16;
        let scroll = if self.scroll_offset > 0 {
            format!(" +{} ", self.scroll_offset)
        } else {
            String::new()
        };
        let focus = if focused { "● " } else { "" };
        let title = format!(" {}{}{} ", focus, self.title, scroll);
        let border_color = if focused {
            Color::White
        } else {
            self.border_color
        };
        let panel = Paragraph::new(text)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color))
                    .title(Span::styled(
                        title,
                        Style::default()
                            .fg(border_color)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .title_alignment(Alignment::Center),
            )
            .scroll((top_line, 0));
        f.render_widget(panel, area);
    }
}

pub fn classify_agent_line(line: &str) -> Option<AgentTerminalRole> {
    let lower = line.to_lowercase();
    if lower.contains("══ turno") && lower.contains("| architect ") {
        Some(AgentTerminalRole::Architect)
    } else if lower.contains("══ turno") && lower.contains("| reviewer ") {
        Some(AgentTerminalRole::Reviewer)
    } else if lower.starts_with("⏳ ia auditora")
        || lower.starts_with("✔ auditora")
        || lower.starts_with("❌ json inválido da auditora")
        || lower.starts_with("❌ resposta inválida da auditora")
    {
        Some(AgentTerminalRole::Auditor)
    } else if lower.starts_with("⏳ ia dev")
        || lower.starts_with("✔ dev")
        || lower.starts_with("📋 plano:")
        || lower.starts_with("diff aprovado")
    {
        Some(AgentTerminalRole::Dev)
    } else {
        None
    }
}
