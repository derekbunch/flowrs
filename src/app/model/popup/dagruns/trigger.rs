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

pub struct TriggerDagRunPopUp {
    pub dag_id: DagId,
    params: Vec<(String, String)>,
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
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), extract_default_value(v)))
                    .collect::<Vec<_>>()
            })
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
        for (key, value) in &self.params {
            let parsed =
                serde_json::from_str(value).unwrap_or(serde_json::Value::String(value.clone()));
            map.insert(key.clone(), parsed);
        }
        Some(serde_json::Value::Object(map))
    }
}

fn extract_default_value(v: &serde_json::Value) -> String {
    if let Some(obj) = v.as_object() {
        if let Some(default) = obj.get("default") {
            return value_to_string(default);
        }
    }
    value_to_string(v)
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
                // Enter on params zone: start editing
                self.editing = true;
                self.cursor_pos = self.params[self.active_param].1.len();
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
        let value = &mut self.params[self.active_param].1;
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

        // Scroll handling
        let visible_rows = params_area.height as usize;
        if self.active_param >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.active_param + 1 - visible_rows;
        }
        if self.active_param < self.scroll_offset {
            self.scroll_offset = self.active_param;
        }

        Clear.render(area, buffer);
        popup_block.render(area, buffer);
        header.render(header_area, buffer);

        // Render param rows
        let max_key_len = self
            .params
            .iter()
            .map(|(k, _)| k.len())
            .max()
            .unwrap_or(0)
            .min(20);

        for (row_idx, (i, (key, value))) in self
            .params
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(visible_rows)
            .enumerate()
        {
            let Some(row_offset) = u16::try_from(row_idx).ok() else {
                break;
            };
            let row_y = params_area.y + row_offset;
            if row_y >= params_area.y + params_area.height {
                break;
            }
            let row_area = Rect::new(
                params_area.x + 1,
                row_y,
                params_area.width.saturating_sub(2),
                1,
            );

            let is_active = i == self.active_param && self.focus == FocusZone::Params;
            let key_style = if is_active {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(TEXT_PRIMARY)
            };

            let truncated_key = if key.len() > max_key_len {
                format!("{}…", &key[..max_key_len.saturating_sub(1)])
            } else {
                format!("{key:max_key_len$}")
            };

            let value_display = if is_active && self.editing {
                let (before, after) = value.split_at(self.cursor_pos.min(value.len()));
                let cursor_char = after.chars().next().unwrap_or(' ');
                let rest = if after.is_empty() {
                    String::new()
                } else {
                    after[cursor_char.len_utf8()..].to_string()
                };
                vec![
                    Span::styled(format!("{truncated_key}: "), key_style),
                    Span::styled(before.to_string(), Style::default().fg(TEXT_PRIMARY)),
                    Span::styled(
                        cursor_char.to_string(),
                        Style::default()
                            .fg(SURFACE_STYLE.bg.unwrap_or(ratatui::style::Color::Black))
                            .bg(TEXT_PRIMARY),
                    ),
                    Span::styled(rest, Style::default().fg(TEXT_PRIMARY)),
                ]
            } else {
                let bracket_style = if is_active {
                    Style::default().fg(ACCENT)
                } else {
                    Style::default().fg(PURPLE_DIM)
                };
                vec![
                    Span::styled(format!("{truncated_key}: "), key_style),
                    Span::styled("[", bracket_style),
                    Span::styled(value.clone(), Style::default().fg(TEXT_PRIMARY)),
                    Span::styled("]", bracket_style),
                ]
            };

            Line::from(value_display).render(row_area, buffer);
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
}
