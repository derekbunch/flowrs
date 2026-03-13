use crossterm::event::KeyCode;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use crate::{
    app::{
        events::custom::FlowrsEvent,
        model::{
            popup::{popup_area, themed_button, SelectedButton},
            Model,
        },
        worker::WorkerMessage,
    },
    ui::theme::{ACCENT, BORDER_STYLE, DEFAULT_STYLE, PURPLE_DIM, SURFACE_STYLE, TEXT_PRIMARY},
};

use crate::airflow::model::common::DagId;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusZone {
    Params,
    Buttons,
}

#[derive(Clone, PartialEq, Eq)]
enum ParamKind {
    /// Free-text input
    Text,
    /// Boolean toggle (true/false)
    Bool,
    /// Fixed set of allowed values (from schema.enum)
    Enum(Vec<String>),
    /// Suggested values but free-text also allowed (from schema.examples)
    Examples(Vec<String>),
}

struct ParamEntry {
    key: String,
    value: String,
    description: Option<String>,
    kind: ParamKind,
}

impl ParamEntry {
    fn has_options(&self) -> bool {
        matches!(self.kind, ParamKind::Enum(_) | ParamKind::Examples(_))
    }

    fn options(&self) -> &[String] {
        match &self.kind {
            ParamKind::Enum(opts) | ParamKind::Examples(opts) => opts,
            _ => &[],
        }
    }
}

pub struct TriggerDagRunPopUp {
    pub dag_id: DagId,
    params: Vec<ParamEntry>,
    active_param: usize,
    editing: bool,
    cursor_pos: usize,
    scroll_offset: usize,
    focus: FocusZone,
    selected_button: SelectedButton,
}

impl TriggerDagRunPopUp {
    pub fn new(dag_id: DagId, raw_params: Option<&serde_json::Value>) -> Self {
        let params = raw_params
            .and_then(|v| v.as_object())
            .map(|obj| obj.iter().map(|(k, v)| extract_param(k, v)).collect())
            .unwrap_or_default();

        Self {
            dag_id,
            params,
            active_param: 0,
            editing: false,
            cursor_pos: 0,
            scroll_offset: 0,
            focus: FocusZone::Params,
            selected_button: SelectedButton::default(),
        }
    }

    fn has_params(&self) -> bool {
        !self.params.is_empty()
    }

    fn build_conf(&self) -> Option<serde_json::Value> {
        if self.params.is_empty() {
            return None;
        }
        let mut map = serde_json::Map::new();
        for entry in &self.params {
            let parsed = serde_json::from_str(&entry.value)
                .unwrap_or(serde_json::Value::String(entry.value.clone()));
            map.insert(entry.key.clone(), parsed);
        }
        Some(serde_json::Value::Object(map))
    }

    fn active_entry(&self) -> &ParamEntry {
        &self.params[self.active_param]
    }

    fn cycle_option(&mut self, forward: bool) {
        let entry = &mut self.params[self.active_param];
        let (ParamKind::Enum(opts) | ParamKind::Examples(opts)) = &entry.kind else {
            return;
        };
        if opts.is_empty() {
            return;
        }
        let current_idx = opts.iter().position(|o| *o == entry.value).unwrap_or(0);
        let next_idx = if forward {
            (current_idx + 1) % opts.len()
        } else {
            current_idx.checked_sub(1).unwrap_or(opts.len() - 1)
        };
        entry.value = opts[next_idx].clone();
    }

    fn toggle_bool(&mut self) {
        let entry = &mut self.params[self.active_param];
        if entry.kind == ParamKind::Bool {
            entry.value = if entry.value == "true" {
                "false"
            } else {
                "true"
            }
            .to_string();
        }
    }
}

fn extract_param(key: &str, v: &serde_json::Value) -> ParamEntry {
    let Some(obj) = v.as_object() else {
        return ParamEntry {
            key: key.to_owned(),
            value: value_to_string(v),
            description: None,
            kind: kind_from_value(v),
        };
    };

    // Airflow wrapped Param class (2.6+): has "__class" key
    if obj.contains_key("__class") {
        let raw_value = obj.get("value").filter(|v| !v.is_null());
        let value = raw_value.map_or_else(String::new, value_to_string);

        let description = obj
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(String::from);

        let schema = obj.get("schema");
        let kind = kind_from_schema(schema, raw_value);

        return ParamEntry {
            key: key.to_owned(),
            value,
            description,
            kind,
        };
    }

    // Some Airflow versions use a "default" key
    if let Some(default) = obj.get("default") {
        return ParamEntry {
            key: key.to_owned(),
            value: value_to_string(default),
            description: None,
            kind: kind_from_value(default),
        };
    }

    ParamEntry {
        key: key.to_owned(),
        value: value_to_string(v),
        description: None,
        kind: ParamKind::Text,
    }
}

fn kind_from_value(v: &serde_json::Value) -> ParamKind {
    if v.is_boolean() {
        ParamKind::Bool
    } else {
        ParamKind::Text
    }
}

fn kind_from_schema(
    schema: Option<&serde_json::Value>,
    raw_value: Option<&serde_json::Value>,
) -> ParamKind {
    let Some(schema) = schema.and_then(|s| s.as_object()) else {
        return raw_value.map_or(ParamKind::Text, kind_from_value);
    };

    // Check for enum (closed set)
    if let Some(values) = schema.get("enum").and_then(|v| v.as_array()) {
        let opts: Vec<String> = values.iter().map(value_to_string).collect();
        if !opts.is_empty() {
            return ParamKind::Enum(opts);
        }
    }

    // Check for examples (open set with suggestions)
    if let Some(values) = schema.get("examples").and_then(|v| v.as_array()) {
        let opts: Vec<String> = values.iter().map(value_to_string).collect();
        if !opts.is_empty() {
            return ParamKind::Examples(opts);
        }
    }

    // Check schema type for booleans
    if schema
        .get("type")
        .and_then(|t| t.as_str())
        .is_some_and(|t| t == "boolean")
    {
        return ParamKind::Bool;
    }

    raw_value.map_or(ParamKind::Text, kind_from_value)
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => v.to_string(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(v).unwrap_or_default()
        }
    }
}

impl Model for TriggerDagRunPopUp {
    fn update(
        &mut self,
        event: &FlowrsEvent,
        _ctx: &crate::app::state::NavigationContext,
    ) -> (Option<FlowrsEvent>, Vec<WorkerMessage>) {
        if let FlowrsEvent::Key(key_event) = event {
            if !self.has_params() {
                return self.update_simple(key_event.code, *key_event);
            }
            return self.update_with_params(key_event.code, *key_event);
        }
        (Some(event.clone()), vec![])
    }
}

impl TriggerDagRunPopUp {
    fn update_simple(
        &mut self,
        code: KeyCode,
        key_event: crossterm::event::KeyEvent,
    ) -> (Option<FlowrsEvent>, Vec<WorkerMessage>) {
        match code {
            KeyCode::Enter => {
                // On Enter, we always return the key event, so the parent can close the popup
                // If Yes is selected, we also return a WorkerMessage to trigger the dag run
                if self.selected_button.is_yes() {
                    return (
                        Some(FlowrsEvent::Key(key_event)),
                        vec![WorkerMessage::TriggerDagRun {
                            dag_id: self.dag_id.clone(),
                            conf: None,
                        }],
                    );
                }
                (Some(FlowrsEvent::Key(key_event)), vec![])
            }
            KeyCode::Char('j' | 'k' | 'h' | 'l')
            | KeyCode::Down
            | KeyCode::Up
            | KeyCode::Left
            | KeyCode::Right => {
                self.selected_button.toggle();
                (None, vec![])
            }
            // On Esc, we always return the key event, so the parent can close the popup, without clearing the dag run
            KeyCode::Char('q') | KeyCode::Esc => (Some(FlowrsEvent::Key(key_event)), vec![]),
            _ => (Some(event_from_key(key_event)), vec![]),
        }
    }

    fn update_with_params(
        &mut self,
        code: KeyCode,
        key_event: crossterm::event::KeyEvent,
    ) -> (Option<FlowrsEvent>, Vec<WorkerMessage>) {
        if self.editing {
            return self.handle_editing(code, key_event);
        }

        match code {
            KeyCode::Esc | KeyCode::Char('q') => (Some(FlowrsEvent::Key(key_event)), vec![]),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    FocusZone::Params => FocusZone::Buttons,
                    FocusZone::Buttons => FocusZone::Params,
                };
                (None, vec![])
            }
            KeyCode::Enter => {
                if self.focus == FocusZone::Buttons {
                    if self.selected_button.is_yes() {
                        return (
                            Some(FlowrsEvent::Key(key_event)),
                            vec![WorkerMessage::TriggerDagRun {
                                dag_id: self.dag_id.clone(),
                                conf: self.build_conf(),
                            }],
                        );
                    }
                    return (Some(FlowrsEvent::Key(key_event)), vec![]);
                }
                match self.active_entry().kind {
                    // Bool: toggle on Enter instead of opening text editor
                    ParamKind::Bool => self.toggle_bool(),
                    // Enum: cycle on Enter (no free-text editing)
                    ParamKind::Enum(_) => self.cycle_option(true),
                    // Text and Examples: open text editor
                    _ => {
                        self.editing = true;
                        self.cursor_pos = self.active_entry().value.len();
                    }
                }
                (None, vec![])
            }
            KeyCode::Char(' ') if self.focus == FocusZone::Params => {
                // Space toggles bools and cycles enums/examples for quick editing
                match self.active_entry().kind {
                    ParamKind::Bool => self.toggle_bool(),
                    ParamKind::Enum(_) | ParamKind::Examples(_) => self.cycle_option(true),
                    ParamKind::Text => {}
                }
                (None, vec![])
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.focus == FocusZone::Params && !self.params.is_empty() {
                    self.active_param = (self.active_param + 1).min(self.params.len() - 1);
                }
                (None, vec![])
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.focus == FocusZone::Params {
                    self.active_param = self.active_param.saturating_sub(1);
                }
                (None, vec![])
            }
            KeyCode::Char('h' | 'l') | KeyCode::Left | KeyCode::Right => {
                if self.focus == FocusZone::Buttons {
                    self.selected_button.toggle();
                }
                (None, vec![])
            }
            _ => (None, vec![]),
        }
    }

    fn handle_editing(
        &mut self,
        code: KeyCode,
        key_event: crossterm::event::KeyEvent,
    ) -> (Option<FlowrsEvent>, Vec<WorkerMessage>) {
        let value = &mut self.params[self.active_param].value;
        match code {
            KeyCode::Esc | KeyCode::Enter => {
                self.editing = false;
            }
            KeyCode::Char(c) => {
                value.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    let prev = value[..self.cursor_pos]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i);
                    value.replace_range(prev..self.cursor_pos, "");
                    self.cursor_pos = prev;
                }
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos = value[..self.cursor_pos]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i);
                }
            }
            KeyCode::Right => {
                if self.cursor_pos < value.len() {
                    self.cursor_pos = value[self.cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map_or(value.len(), |(i, _)| self.cursor_pos + i);
                }
            }
            _ => {}
        }
        let _ = key_event;
        (None, vec![])
    }
}

fn event_from_key(key_event: crossterm::event::KeyEvent) -> FlowrsEvent {
    FlowrsEvent::Key(key_event)
}

impl Widget for &mut TriggerDagRunPopUp {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        if self.has_params() {
            self.render_with_params(area, buffer);
        } else {
            self.render_simple(area, buffer);
        }
    }
}

const GHOST_STYLE: Style = Style {
    fg: Some(PURPLE_DIM),
    ..DEFAULT_STYLE
};

impl TriggerDagRunPopUp {
    fn render_simple(&mut self, area: Rect, buffer: &mut Buffer) {
        // Smaller popup: 40% width, auto height
        let area = popup_area(area, 40, 30);

        let popup_block = Block::default()
            .border_type(BorderType::Rounded)
            .borders(Borders::ALL)
            .border_style(BORDER_STYLE)
            .style(SURFACE_STYLE);

        // Use inner area for content layout to avoid overlapping the border
        let inner = popup_block.inner(area);

        let [_, header, options, _] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .flex(Flex::Center)
        .areas(inner);

        let text = Paragraph::new("Trigger a new DAG Run?")
            .style(DEFAULT_STYLE)
            .centered();

        let [_, yes, _, no, _] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Fill(1),
        ])
        .areas(options);

        let yes_btn = themed_button("Yes", self.selected_button.is_yes());
        let no_btn = themed_button("No", !self.selected_button.is_yes());

        Clear.render(area, buffer);
        popup_block.render(area, buffer);
        text.render(header, buffer);
        yes_btn.render(yes, buffer);
        no_btn.render(no, buffer);
    }

    fn render_with_params(&mut self, area: Rect, buffer: &mut Buffer) {
        let area = popup_area(area, 60, 70);

        let popup_block = Block::default()
            .border_type(BorderType::Rounded)
            .borders(Borders::ALL)
            .border_style(BORDER_STYLE)
            .style(SURFACE_STYLE)
            .title(" Trigger DAG Run ");

        let inner = popup_block.inner(area);

        // Each param gets 2 rows: value line + description/options ghost line
        let [header_area, params_area, _, buttons_area, _] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(inner);

        let header = Paragraph::new("Edit parameters and confirm:")
            .style(DEFAULT_STYLE)
            .centered();

        let rows_per_param: usize = 2;
        let visible_params = params_area.height as usize / rows_per_param;

        // Scroll handling
        if self.active_param >= self.scroll_offset + visible_params {
            self.scroll_offset = self.active_param + 1 - visible_params;
        }
        if self.active_param < self.scroll_offset {
            self.scroll_offset = self.active_param;
        }

        Clear.render(area, buffer);
        popup_block.render(area, buffer);
        header.render(header_area, buffer);

        let max_key_len = self.params.iter().map(|e| e.key.len()).max().unwrap_or(0);

        for (row_idx, (i, entry)) in self
            .params
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(visible_params)
            .enumerate()
        {
            let Some(row_offset) = u16::try_from(row_idx * rows_per_param).ok() else {
                break;
            };
            let row_y = params_area.y + row_offset;
            if row_y + 1 >= params_area.y + params_area.height {
                break;
            }
            let row_area = Rect::new(
                params_area.x + 1,
                row_y,
                params_area.width.saturating_sub(2),
                1,
            );
            let ghost_area = Rect::new(
                params_area.x + 1,
                row_y + 1,
                params_area.width.saturating_sub(2),
                1,
            );

            let is_active = i == self.active_param && self.focus == FocusZone::Params;
            let key_style = if is_active {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(TEXT_PRIMARY)
            };

            let truncated_key = if entry.key.len() > max_key_len {
                format!("{}…", &entry.key[..max_key_len.saturating_sub(1)])
            } else {
                format!("{:max_key_len$}", entry.key)
            };

            let value_line = self.render_value_line(entry, &truncated_key, key_style, is_active);
            value_line.render(row_area, buffer);

            // Ghost line: description, or options list, or kind hint
            let ghost_line = render_ghost_line(entry, max_key_len, is_active);
            if let Some(line) = ghost_line {
                line.render(ghost_area, buffer);
            }
        }

        // Buttons
        let [_, yes, _, no, _] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Fill(1),
        ])
        .areas(buttons_area);

        let btn_focus = self.focus == FocusZone::Buttons;
        let yes_btn = themed_button("Yes", btn_focus && self.selected_button.is_yes());
        let no_btn = themed_button("No", btn_focus && !self.selected_button.is_yes());

        yes_btn.render(yes, buffer);
        no_btn.render(no, buffer);
    }

    fn render_value_line(
        &self,
        entry: &ParamEntry,
        truncated_key: &str,
        key_style: Style,
        is_active: bool,
    ) -> Line<'static> {
        if is_active && self.editing {
            return self.render_editing_line(entry, truncated_key, key_style);
        }

        let bracket_style = if is_active {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(PURPLE_DIM)
        };

        let mut spans = vec![Span::styled(format!("{truncated_key}: "), key_style)];

        match &entry.kind {
            ParamKind::Bool => {
                let (symbol, color) = if entry.value == "true" {
                    ("✓ true", ACCENT)
                } else {
                    ("✗ false", PURPLE_DIM)
                };
                spans.push(Span::styled(symbol.to_string(), Style::default().fg(color)));
                if is_active {
                    spans.push(Span::styled(" <Space> toggle", GHOST_STYLE));
                }
            }
            ParamKind::Enum(opts) => {
                let current_idx = opts.iter().position(|o| *o == entry.value);
                spans.push(Span::styled("[", bracket_style));
                spans.push(Span::styled(
                    entry.value.clone(),
                    Style::default().fg(TEXT_PRIMARY),
                ));
                spans.push(Span::styled("]", bracket_style));
                if let Some(idx) = current_idx {
                    spans.push(Span::styled(
                        format!(" ({}/{})", idx + 1, opts.len()),
                        GHOST_STYLE,
                    ));
                }
                if is_active {
                    spans.push(Span::styled(" <Space> cycle", GHOST_STYLE));
                }
            }
            _ => {
                spans.push(Span::styled("[", bracket_style));
                spans.push(Span::styled(
                    entry.value.clone(),
                    Style::default().fg(TEXT_PRIMARY),
                ));
                spans.push(Span::styled("]", bracket_style));
                if is_active && entry.has_options() {
                    spans.push(Span::styled(" <Space> cycle", GHOST_STYLE));
                }
            }
        }

        Line::from(spans)
    }

    fn render_editing_line(
        &self,
        entry: &ParamEntry,
        truncated_key: &str,
        key_style: Style,
    ) -> Line<'static> {
        let (before, after) = entry.value.split_at(self.cursor_pos.min(entry.value.len()));
        let cursor_char = after.chars().next().unwrap_or(' ');
        let rest = if after.is_empty() {
            String::new()
        } else {
            after[cursor_char.len_utf8()..].to_string()
        };
        Line::from(vec![
            Span::styled(format!("{truncated_key}: "), key_style),
            Span::styled(before.to_string(), Style::default().fg(TEXT_PRIMARY)),
            Span::styled(
                cursor_char.to_string(),
                Style::default()
                    .fg(SURFACE_STYLE.bg.unwrap_or(ratatui::style::Color::Black))
                    .bg(TEXT_PRIMARY),
            ),
            Span::styled(rest, Style::default().fg(TEXT_PRIMARY)),
        ])
    }
}

fn render_ghost_line(
    entry: &ParamEntry,
    key_padding: usize,
    is_active: bool,
) -> Option<Line<'static>> {
    let padding = " ".repeat(key_padding + 2); // align with value after "key: "

    // Description always takes priority
    if let Some(desc) = &entry.description {
        return Some(Line::from(Span::styled(
            format!("{padding}{desc}"),
            GHOST_STYLE,
        )));
    }

    // For active entries with options, show the option list
    if is_active {
        let opts = entry.options();
        if !opts.is_empty() {
            let opts_display: Vec<String> = opts
                .iter()
                .map(|o| {
                    if *o == entry.value {
                        format!("[{o}]")
                    } else {
                        o.clone()
                    }
                })
                .collect();
            return Some(Line::from(Span::styled(
                format!("{padding}{}", opts_display.join(" | ")),
                GHOST_STYLE,
            )));
        }
    }

    None
}
