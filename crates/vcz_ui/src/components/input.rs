use crate::{action::Action, app_style::AppStyle};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect}, style::*, text::{Line, Text}, widgets::*, Frame
};

use super::{Component, HandleActionResponse};

#[derive(Default, Clone, Copy, PartialEq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
}

pub struct Input<'a> {
    pub focused: bool,
    pub block: Block<'a>,
    pub value: String,
    pub mode: Mode,
    pub cursor_position: usize,
    pub style: AppStyle,
}

impl<'a> Input<'a> {
    pub fn new(style: AppStyle) -> Self {
        let block = Block::default().borders(Borders::ALL).title("Input");

        Self {
            style,
            block,
            value: "".into(),
            mode: Mode::default(),
            focused: false,
            cursor_position: 0,
        }
    }

    pub fn value(mut self, value: String) -> Self {
        self.cursor_position = value.chars().count();
        self.value = value;
        self
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = block;
        self
    }

    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
}

impl<'a> Input<'a> {
    fn delete_char(&mut self) {
        let is_not_cursor_leftmost = self.cursor_position != 0;
        if is_not_cursor_leftmost {
            // Method "remove" is not used on the saved text for deleting the
            // selected char. Reason: Using remove on String works
            // on bytes instead of the chars. Using remove would
            // require special care because of char boundaries.
            let current_index = self.cursor_position;
            let from_left_to_current_index = current_index - 1;

            // Getting all characters before the selected character.
            let before_char_to_delete =
                self.value.chars().take(from_left_to_current_index);
            // Getting all characters after selected character.
            let after_char_to_delete = self.value.chars().skip(current_index);

            // Put all characters together except the selected one.
            // By leaving the selected one out, it is forgotten and therefore
            // deleted.
            self.value =
                before_char_to_delete.chain(after_char_to_delete).collect();
            self.move_cursor_left();
        }
    }
    fn clamp_cursor(&self, new_cursor_pos: usize) -> usize {
        new_cursor_pos.clamp(0, self.value.len())
    }

    fn reset_cursor(&mut self) {
        self.cursor_position = 0;
    }

    fn submit_message(&mut self) {
        self.value.clear();
        self.reset_cursor();
    }
    fn move_cursor_left(&mut self) {
        let cursor_moved_left = self.cursor_position.saturating_sub(1);
        self.cursor_position = self.clamp_cursor(cursor_moved_left);
    }

    fn move_cursor_right(&mut self) {
        let cursor_moved_right = self.cursor_position.saturating_add(1);
        self.cursor_position = self.clamp_cursor(cursor_moved_right);
    }

    fn enter_char(&mut self, new_char: char) {
        self.value.insert(self.cursor_position, new_char);
        self.move_cursor_right();
    }
}

impl<'a> Component for Input<'a> {
    fn draw(&mut self, f: &mut Frame, rect: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3)])
            .split(rect);

        let mut border_style = Style::default().fg(Color::Gray);

        if self.focused {
            border_style = self.style.highlight_fg;
        }

        let (msg, style) = match self.mode {
            Mode::Normal => (
                vec![
                    "Press ".into(),
                    "q".bold(),
                    " to exit, ".into(),
                    "i".bold(),
                    " to start editing.".bold(),
                ],
                Style::default().add_modifier(Modifier::RAPID_BLINK),
            ),
            Mode::Insert => (
                vec![
                    "Press ".into(),
                    "Esc".bold(),
                    " to stop editing, ".into(),
                    "Enter".bold(),
                    " to save".into(),
                ],
                Style::default(),
            ),
        };
        let text = Text::from(Line::from(msg));
        let text = text.patch_style(style);
        let help_message = Paragraph::new(text);

        // render help message
        f.render_widget(help_message, chunks[0]);

        let input = Paragraph::new(self.value.as_str())
            .style(match self.mode {
                Mode::Normal => Style::default(),
                Mode::Insert => Style::default().fg(Color::Cyan),
            })
            .block(self.block.clone().border_style(border_style));

        // render input
        f.render_widget(input, chunks[1]);
    }

    fn handle_action(&mut self, action: &Action) -> HandleActionResponse {
        match self.mode {
            Mode::Normal if let Action::Key(k) = action => match k.code {
                KeyCode::Char('i') => self.mode = Mode::Insert,
                _ => {}
            },
            Mode::Insert if let Action::Key(k) = action => match k.code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    return HandleActionResponse::Ignore;
                }

                KeyCode::Char(c) => {
                    self.enter_char(c);
                    return HandleActionResponse::Ignore;
                }
                KeyCode::Backspace => self.delete_char(),
                KeyCode::Left => self.move_cursor_left(),
                KeyCode::Right => self.move_cursor_right(),
                KeyCode::Enter => self.submit_message(),
                _ => {}
            },
            _ => {}
        }
        HandleActionResponse::default()
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}
