mod render;

use crossterm::event::KeyCode;
use ratatui::widgets::ScrollbarState;
use unicode_width::UnicodeWidthStr;

use crate::airflow::model::common::{Log, OpenItem};
use crate::app::events::custom::FlowrsEvent;
use crate::app::worker::WorkerMessage;

use super::popup::error::ErrorPopup;
use super::Model;

/// Represents the log viewer's scroll behavior.
///
/// Eliminates the invalid state where `follow_mode = true` but `vertical_scroll`
/// points somewhere other than the bottom.
#[derive(Default)]
pub enum ScrollMode {
    /// Automatically scroll to bottom when new content arrives (tail mode).
    /// The scroll position is computed from the content length at render time.
    #[default]
    Following,
    /// User is manually scrolling at a fixed position.
    Manual { position: usize },
}

impl ScrollMode {
    /// Returns the concrete scroll position for the rendered content height.
    pub(crate) fn position(&self, content_height: usize, viewport_height: usize) -> usize {
        let bottom = bottom_scroll_position(content_height, viewport_height);
        match self {
            ScrollMode::Following => bottom,
            ScrollMode::Manual { position } => (*position).min(bottom),
        }
    }

    fn is_following(&self) -> bool {
        matches!(self, ScrollMode::Following)
    }
}

pub(crate) fn bottom_scroll_position(content_height: usize, viewport_height: usize) -> usize {
    content_height.saturating_sub(viewport_height)
}

pub(crate) fn wrapped_line_count(content: &str, line_width: usize) -> usize {
    if line_width == 0 {
        return content.lines().count();
    }

    content
        .lines()
        .map(|line| {
            let rendered_width = UnicodeWidthStr::width(line);
            rendered_width.saturating_sub(1) / line_width + 1
        })
        .sum()
}

pub struct LogModel {
    pub all: Vec<Log>,
    pub current: usize,
    pub error_popup: Option<ErrorPopup>,
    ticks: u32,
    poll_tick_multiplier: u32,
    pub(crate) scroll_mode: ScrollMode,
    pub(crate) vertical_scroll_state: ScrollbarState,
    pub(crate) last_scroll_position: usize,
    pub(crate) last_bottom_scroll_position: usize,
    pub(crate) last_viewport_height: usize,
    pending_g: bool,
}

impl Default for LogModel {
    fn default() -> Self {
        Self {
            all: Vec::new(),
            current: 0,
            error_popup: None,
            ticks: 0,
            poll_tick_multiplier: 10,
            scroll_mode: ScrollMode::default(),
            vertical_scroll_state: ScrollbarState::default(),
            last_scroll_position: 0,
            last_bottom_scroll_position: 0,
            last_viewport_height: 1,
            pending_g: false,
        }
    }
}

impl LogModel {
    pub fn new(poll_tick_multiplier: u32) -> Self {
        Self {
            poll_tick_multiplier,
            ..Self::default()
        }
    }

    /// Reset scroll to follow mode (used when navigating to a new context)
    pub fn reset_scroll(&mut self) {
        self.scroll_mode = ScrollMode::Following;
    }

    /// Update the logs content. When in follow mode, the scroll position
    /// will automatically track the bottom at render time.
    pub fn update_logs(&mut self, logs: Vec<Log>) {
        self.all = logs;
    }

    fn scroll_up(&mut self, amount: usize) {
        self.scroll_mode = ScrollMode::Manual {
            position: self.last_scroll_position.saturating_sub(amount.max(1)),
        };
    }

    fn scroll_down(&mut self, amount: usize) {
        let new_pos = self.last_scroll_position.saturating_add(amount.max(1));
        if new_pos >= self.last_bottom_scroll_position {
            self.scroll_mode = ScrollMode::Following;
        } else {
            self.scroll_mode = ScrollMode::Manual { position: new_pos };
        }
    }

    fn half_page_height(&self) -> usize {
        (self.last_viewport_height / 2).max(1)
    }
}

impl Model for LogModel {
    fn update(
        &mut self,
        event: &FlowrsEvent,
        ctx: &crate::app::state::NavigationContext,
    ) -> (Option<FlowrsEvent>, Vec<WorkerMessage>) {
        match event {
            FlowrsEvent::Tick => {
                self.ticks += 1;
                if !self.ticks.is_multiple_of(self.poll_tick_multiplier) {
                    return (Some(FlowrsEvent::Tick), vec![]);
                }
                if let (Some(dag_id), Some(dag_run_id), Some(task_id), Some(task_try)) = (
                    ctx.dag_id(),
                    ctx.dag_run_id(),
                    ctx.task_id(),
                    ctx.task_try(),
                ) {
                    log::debug!("Updating task logs for dag_run_id: {dag_run_id}");
                    return (
                        Some(FlowrsEvent::Tick),
                        vec![WorkerMessage::UpdateTaskLogs {
                            dag_id: dag_id.clone(),
                            dag_run_id: dag_run_id.clone(),
                            task_id: task_id.clone(),
                            task_try,
                        }],
                    );
                }
                return (Some(FlowrsEvent::Tick), vec![]);
            }
            FlowrsEvent::Key(key) => {
                if let Some(_error_popup) = &mut self.error_popup {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            self.error_popup = None;
                        }
                        _ => (),
                    }
                    return (None, vec![]);
                }
                // Clear pending 'g' on any key that is not 'g' to ensure gg requires consecutive presses
                if key.code != KeyCode::Char('g') {
                    self.pending_g = false;
                }
                match key.code {
                    KeyCode::Char('l') | KeyCode::Right => {
                        if !self.all.is_empty() && self.current < self.all.len() - 1 {
                            self.current += 1;
                        }
                    }
                    KeyCode::Char('h') | KeyCode::Left => {
                        if self.all.is_empty() || self.current == 0 {
                            // Navigate back to previous panel
                            return (Some(FlowrsEvent::Key(*key)), vec![]);
                        }
                        self.current -= 1;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.scroll_down(1);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.scroll_up(1);
                    }
                    KeyCode::Char('u') => {
                        self.scroll_up(self.half_page_height());
                    }
                    KeyCode::Char('d') => {
                        self.scroll_down(self.half_page_height());
                    }
                    KeyCode::Char('b') => {
                        self.scroll_up(self.last_viewport_height);
                    }
                    KeyCode::Char('f') => {
                        self.scroll_down(self.last_viewport_height);
                    }
                    KeyCode::Char('o') => {
                        if self.all.get(self.current % self.all.len()).is_some() {
                            if let (Some(dag_id), Some(dag_run_id), Some(task_id)) =
                                (ctx.dag_id(), ctx.dag_run_id(), ctx.task_id())
                            {
                                return (
                                    Some(FlowrsEvent::Key(*key)),
                                    vec![WorkerMessage::OpenItem(OpenItem::Log {
                                        dag_id: dag_id.clone(),
                                        dag_run_id: dag_run_id.clone(),
                                        task_id: task_id.clone(),
                                        #[allow(clippy::cast_possible_truncation)]
                                        task_try: (self.current + 1) as u32,
                                    })],
                                );
                            }
                        }
                    }
                    KeyCode::Char('G') => {
                        self.scroll_mode = ScrollMode::Following;
                    }
                    KeyCode::Char('F') => {
                        // Toggle follow mode
                        if self.scroll_mode.is_following() {
                            self.scroll_mode = ScrollMode::Manual {
                                position: self.last_scroll_position,
                            };
                        } else {
                            self.scroll_mode = ScrollMode::Following;
                        }
                    }
                    KeyCode::Char('g') => {
                        // gg: go to top of log
                        if self.pending_g {
                            self.scroll_mode = ScrollMode::Manual { position: 0 };
                            self.pending_g = false;
                        } else {
                            self.pending_g = true;
                        }
                    }

                    _ => return (Some(FlowrsEvent::Key(*key)), vec![]), // if no match, return the event
                }
            }
            FlowrsEvent::Mouse | FlowrsEvent::FocusGained | FlowrsEvent::FocusLost => (),
        }

        (None, vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::{bottom_scroll_position, wrapped_line_count, LogModel, ScrollMode};

    #[test]
    fn follow_mode_uses_bottom_visible_offset() {
        assert_eq!(ScrollMode::Following.position(100, 20), 80);
        assert_eq!(ScrollMode::Following.position(10, 20), 0);
    }

    #[test]
    fn manual_mode_is_clamped_to_bottom_offset() {
        assert_eq!(ScrollMode::Manual { position: 12 }.position(100, 20), 12);
        assert_eq!(ScrollMode::Manual { position: 90 }.position(100, 20), 80);
    }

    #[test]
    fn bottom_scroll_offset_saturates_when_content_fits() {
        assert_eq!(bottom_scroll_position(5, 20), 0);
        assert_eq!(bottom_scroll_position(20, 20), 0);
        assert_eq!(bottom_scroll_position(21, 20), 1);
    }

    #[test]
    fn wrapped_line_count_counts_visual_rows() {
        assert_eq!(wrapped_line_count("12345\n1234567890\n12345678901", 10), 4);
        assert_eq!(wrapped_line_count("", 10), 0);
        assert_eq!(wrapped_line_count("abc", 0), 1);
    }

    #[test]
    fn fast_scroll_uses_viewport_relative_amounts() {
        let mut model = LogModel {
            last_scroll_position: 50,
            last_bottom_scroll_position: 100,
            last_viewport_height: 20,
            ..LogModel::default()
        };

        model.scroll_up(model.half_page_height());
        assert_manual_position(&model.scroll_mode, 40);

        model.last_scroll_position = 50;
        model.scroll_down(model.half_page_height());
        assert_manual_position(&model.scroll_mode, 60);

        model.last_scroll_position = 50;
        model.scroll_up(model.last_viewport_height);
        assert_manual_position(&model.scroll_mode, 30);

        model.last_scroll_position = 50;
        model.scroll_down(model.last_viewport_height);
        assert_manual_position(&model.scroll_mode, 70);
    }

    #[test]
    fn fast_scroll_down_resumes_follow_at_bottom() {
        let mut model = LogModel {
            last_scroll_position: 95,
            last_bottom_scroll_position: 100,
            last_viewport_height: 20,
            ..LogModel::default()
        };

        model.scroll_down(model.half_page_height());

        assert!(model.scroll_mode.is_following());
    }

    fn assert_manual_position(scroll_mode: &ScrollMode, expected: usize) {
        match scroll_mode {
            ScrollMode::Manual { position } => assert_eq!(*position, expected),
            ScrollMode::Following => panic!("expected manual scroll position {expected}"),
        }
    }
}
