use std::{marker::PhantomData, sync::Arc};

use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout}, Frame
};

use crate::{
    action::Action, app::AppCtx, components::{torrent_list::TorrentList, Component, HandleActionResponse}, tui::Event
};

use super::Page;

/// This is the main "page" of the UI, a list of torrents
/// with their state, such as: download rate, name, percentage, etc.
///
/// It handles it's own keybindings and communicate with the main [`UI`]
/// through mpsc.
pub struct Home<'a> {
    pub focused: usize,
    pub layout: Layout,
    pub components: Vec<Box<dyn Component>>,
    phantom: PhantomData<&'a i32>,
    ctx: Arc<AppCtx>,
}

impl<'a> Home<'a> {
    pub fn new(ctx: Arc<AppCtx>) -> Self {
        let torrent_list: Box<dyn Component> = Box::new(TorrentList::new(ctx.clone()));
        let components = vec![torrent_list];

        Self {
            phantom: PhantomData,
            layout: Layout::new(
                Direction::Vertical,
                Constraint::from_percentages([100]),
            ),
            components,
            focused: 0,
            ctx,
        }
    }
    async fn quit(&self) {
        self.ctx.tx.send(Action::Quit).unwrap();
    }
}

impl<'a> Page for Home<'a> {
    fn get_action(&self, event: Event) -> Action {
        match event {
            Event::Error => Action::None,
            Event::Tick => Action::Tick,
            Event::Render => Action::Render,
            Event::Key(key) => Action::Key(key),
            Event::Quit => Action::Quit,
            _ => Action::None,
        }
    }

    fn handle_action(&mut self, action: Action) {
        for component in &mut self.components {
            if let HandleActionResponse::Handle =
                component.handle_action(&action)
            {
                if let Action::Key(key) = action {
                    if let KeyCode::Char('q') = key.code {
                        self.ctx.tx.send(Action::Quit).unwrap();
                    }
                }
            }
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        // areas.len() must match the quantity of components
        let areas = self.layout.split(f.size());

        for (component, area) in
            self.components.iter_mut().zip(areas.into_iter())
        {
            component.draw(f, *area);
        }
    }

    fn focus_next(&mut self) {
        if self.components.len() <= self.focused + 1 {
            self.focused += 1;
        }
    }

    fn focus_prev(&mut self) {
        if self.focused > 0 && !self.components.is_empty() {
            self.focused -= 1;
        }
    }
}
