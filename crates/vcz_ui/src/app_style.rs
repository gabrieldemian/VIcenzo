use ratatui::style::{Color, Style};

#[derive(Clone, Debug)]
pub struct AppStyle {
    pub base: Style,
    pub highlight_bg: Style,
    pub highlight_fg: Style,
    pub success: Style,
    pub error: Style,
    pub warning: Style,
}

impl Default for AppStyle {
    fn default() -> Self {
        Self::new()
    }
}

impl AppStyle {
    pub fn new() -> Self {
        AppStyle {
            base: Style::default().fg(Color::Gray),
            highlight_bg: Style::default()
                .bg(Color::LightBlue)
                .fg(Color::DarkGray),
            highlight_fg: Style::default().fg(Color::LightBlue),
            success: Style::default().fg(Color::LightGreen),
            error: Style::default().fg(Color::Red),
            warning: Style::default().fg(Color::Yellow),
        }
    }
}
