pub mod input;
pub mod torrent_list;

use ratatui::{layout::Rect, Frame};

/// Each component, after calling "handle_action", will either let the page
/// caller handle the action again, or ignore it. The component is the first one
/// to handle it.
#[derive(Clone, Copy, Debug, Default)]
pub enum HandleActionResponse {
    Ignore,
    #[default]
    Handle,
}

pub trait Component {
    fn handle_action(
        &mut self,
        action: &crate::action::Action,
    ) -> HandleActionResponse;
    fn draw(&mut self, f: &mut Frame, rect: Rect);
    fn focus(&mut self);
    fn unfocus(&mut self);
}
