use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Text},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, StatefulWidget,
        Tabs, Widget, Wrap,
    },
};

use crate::ui::theme::theme;

use super::{bottom_scroll_position, wrapped_line_count, LogModel};

impl Widget for &mut LogModel {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let t = theme();

        if self.all.is_empty() {
            Paragraph::new("No logs available")
                .style(t.default_style)
                .block(
                    Block::default()
                        .border_type(BorderType::Rounded)
                        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                        .border_style(t.border_style),
                )
                .render(area, buffer);
            return;
        }

        let tab_titles = (0..self.all.len())
            .map(|i| format!("Task {}", i + 1))
            .collect::<Vec<String>>();

        let tabs = Tabs::new(tab_titles)
            .block(
                Block::default()
                    .border_type(BorderType::Rounded)
                    .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                    .border_style(t.border_style),
            )
            .select(self.current % self.all.len())
            .highlight_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD))
            .style(t.default_style);

        // Render the tabs
        tabs.render(area, buffer);

        // Define the layout for content under the tabs
        let chunks = Layout::default()
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        if let Some(log) = self.all.get(self.current % self.all.len()) {
            let mut content = Text::default();
            for line in log.content.lines() {
                content.push_line(Line::raw(line));
            }

            let log_area = chunks[1];
            let viewport_height = log_area.height.saturating_sub(2) as usize;
            let line_width = log_area.width.saturating_sub(2) as usize;
            let rendered_line_count = wrapped_line_count(&log.content, line_width);
            self.last_viewport_height = viewport_height.max(1);
            self.last_bottom_scroll_position =
                bottom_scroll_position(rendered_line_count, viewport_height);
            let scroll_pos = self
                .scroll_mode
                .position(rendered_line_count, viewport_height);
            self.last_scroll_position = scroll_pos;
            self.vertical_scroll_state = self
                .vertical_scroll_state
                .content_length(rendered_line_count)
                .position(scroll_pos);

            #[allow(clippy::cast_possible_truncation)]
            let paragraph = Paragraph::new(content)
                .block(
                    Block::default()
                        .border_type(BorderType::Plain)
                        .borders(Borders::ALL)
                        .title(" Content ")
                        .title_bottom(if self.scroll_mode.is_following() {
                            " [F]ollow: ON - auto-scrolling "
                        } else {
                            " [F]ollow: OFF - press G to resume "
                        })
                        .border_style(t.border_style)
                        .title_style(t.title_style),
                )
                .wrap(Wrap { trim: true })
                .style(t.default_style)
                .scroll((scroll_pos as u16, 0));

            // Render the selected log's content
            paragraph.render(chunks[1], buffer);

            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));

            scrollbar.render(chunks[1], buffer, &mut self.vertical_scroll_state);
        }

        if let Some(error_popup) = &self.error_popup {
            error_popup.render(area, buffer);
        }
    }
}
