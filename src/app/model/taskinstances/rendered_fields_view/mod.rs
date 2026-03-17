mod render;

use crossterm::event::KeyCode;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ScrollbarState;

use crate::ui::theme::theme;

pub struct RenderedFieldsView {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) vertical_scroll: usize,
    pub(crate) vertical_scroll_state: ScrollbarState,
    event_buffer: Vec<KeyCode>,
}

impl RenderedFieldsView {
    pub fn new(rendered_fields: &serde_json::Value) -> Self {
        let lines = fields_to_lines(rendered_fields);
        let content_length = lines.len();
        Self {
            lines,
            vertical_scroll: 0,
            vertical_scroll_state: ScrollbarState::default().content_length(content_length),
            event_buffer: Vec::new(),
        }
    }

    /// Handle a key event. Returns `true` if the view should be closed.
    pub fn update(&mut self, key_code: KeyCode) -> bool {
        match key_code {
            KeyCode::Esc | KeyCode::Char('q' | 'r') | KeyCode::Enter => return true,
            KeyCode::Down | KeyCode::Char('j') => {
                self.vertical_scroll = self.vertical_scroll.saturating_add(1);
                self.vertical_scroll_state =
                    self.vertical_scroll_state.position(self.vertical_scroll);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.vertical_scroll = self.vertical_scroll.saturating_sub(1);
                self.vertical_scroll_state =
                    self.vertical_scroll_state.position(self.vertical_scroll);
            }
            KeyCode::Char('G') => {
                self.vertical_scroll = self.lines.len().saturating_sub(1);
                self.vertical_scroll_state =
                    self.vertical_scroll_state.position(self.vertical_scroll);
            }
            KeyCode::Char('g') => {
                if let Some(KeyCode::Char('g')) = self.event_buffer.pop() {
                    self.vertical_scroll = 0;
                    self.vertical_scroll_state = self.vertical_scroll_state.position(0);
                } else {
                    self.event_buffer.push(key_code);
                }
            }
            _ => {}
        }
        false
    }
}

fn fields_to_lines(value: &serde_json::Value) -> Vec<Line<'static>> {
    let t = theme();
    let mut lines = Vec::new();

    let Some(obj) = value.as_object() else {
        lines.push(Line::from(Span::styled(
            "No rendered fields available",
            Style::default().fg(t.text_primary),
        )));
        return lines;
    };

    let key_style = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let string_style = Style::default().fg(t.state_success);
    let number_style = Style::default().fg(t.state_running);
    let bool_style = Style::default().fg(t.state_up_for_retry);
    let null_style = Style::default()
        .fg(t.purple_dim)
        .add_modifier(Modifier::ITALIC);

    let mut first = true;
    for (key, val) in obj {
        if !first {
            lines.push(Line::from(""));
        }
        first = false;

        // Key header
        lines.push(Line::from(vec![Span::styled(
            format!("  {key}"),
            key_style,
        )]));

        // Separator line under key
        let separator = "─".repeat(key.len() + 2);
        lines.push(Line::from(Span::styled(
            format!("  {separator}"),
            Style::default().fg(t.purple_dim),
        )));

        // Value
        render_value(
            val,
            &mut lines,
            4,
            &string_style,
            &number_style,
            &bool_style,
            &null_style,
        );
    }

    lines
}

fn render_value(
    value: &serde_json::Value,
    lines: &mut Vec<Line<'static>>,
    indent: usize,
    string_style: &Style,
    number_style: &Style,
    bool_style: &Style,
    null_style: &Style,
) {
    let pad = " ".repeat(indent);
    match value {
        serde_json::Value::Null => {
            lines.push(Line::from(Span::styled(format!("{pad}null"), *null_style)));
        }
        serde_json::Value::Bool(b) => {
            lines.push(Line::from(Span::styled(format!("{pad}{b}"), *bool_style)));
        }
        serde_json::Value::Number(n) => {
            lines.push(Line::from(Span::styled(format!("{pad}{n}"), *number_style)));
        }
        serde_json::Value::String(s) => {
            // Multi-line strings get each line on its own row
            if s.contains('\n') {
                for line in s.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("{pad}{line}"),
                        *string_style,
                    )));
                }
            } else {
                lines.push(Line::from(Span::styled(format!("{pad}{s}"), *string_style)));
            }
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                lines.push(Line::from(Span::styled(format!("{pad}[]"), *null_style)));
                return;
            }
            lines.push(Line::from(Span::styled(
                format!("{pad}["),
                Style::default().fg(Color::DarkGray),
            )));
            for item in arr {
                render_value(
                    item,
                    lines,
                    indent + 2,
                    string_style,
                    number_style,
                    bool_style,
                    null_style,
                );
            }
            lines.push(Line::from(Span::styled(
                format!("{pad}]"),
                Style::default().fg(Color::DarkGray),
            )));
        }
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                lines.push(Line::from(Span::styled(format!("{pad}{{}}"), *null_style)));
                return;
            }
            let t = theme();
            let nested_key_style = Style::default().fg(t.accent);
            lines.push(Line::from(Span::styled(
                format!("{pad}{{"),
                Style::default().fg(Color::DarkGray),
            )));
            for (k, v) in map {
                // For simple values, put key: value on one line
                if is_simple(v) {
                    let val_span =
                        simple_value_span(v, string_style, number_style, bool_style, null_style);
                    lines.push(Line::from(vec![
                        Span::styled(format!("{pad}  {k}: "), nested_key_style),
                        val_span,
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("{pad}  {k}:"),
                        nested_key_style,
                    )));
                    render_value(
                        v,
                        lines,
                        indent + 4,
                        string_style,
                        number_style,
                        bool_style,
                        null_style,
                    );
                }
            }
            lines.push(Line::from(Span::styled(
                format!("{pad}}}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
}

fn is_simple(value: &serde_json::Value) -> bool {
    matches!(
        value,
        serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_)
    )
}

fn simple_value_span(
    value: &serde_json::Value,
    string_style: &Style,
    number_style: &Style,
    bool_style: &Style,
    null_style: &Style,
) -> Span<'static> {
    match value {
        serde_json::Value::Null => Span::styled("null", *null_style),
        serde_json::Value::Bool(b) => Span::styled(b.to_string(), *bool_style),
        serde_json::Value::Number(n) => Span::styled(n.to_string(), *number_style),
        serde_json::Value::String(s) => Span::styled(s.clone(), *string_style),
        _ => Span::raw(""),
    }
}
