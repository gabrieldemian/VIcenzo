use ratatui::Frame;

use crate::{action::Action, tui::Event};

pub mod home;

pub trait Page {
    fn draw(&mut self, f: &mut Frame);
    fn handle_action(&mut self, action: Action);
    /// get an app event and transform into a page action
    fn get_action(&self, event: Event) -> Action;
    fn focus_next(&mut self);
    fn focus_prev(&mut self);
}
