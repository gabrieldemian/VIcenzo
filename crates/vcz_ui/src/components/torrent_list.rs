use std::sync::Arc;

use crossterm::event::KeyCode;
use hashbrown::HashMap;
use ratatui::{prelude::*, widgets::*};
use vincenzo::{
    torrent::{TorrentState, TorrentStatus}, utils::to_human_readable
};

use crate::{action::Action, app::AppCtx, utils::centered_rect};

use super::{
    input::{Input, Mode}, Component, HandleActionResponse
};

pub struct TorrentList<'a> {
    pub focused: bool,
    pub state: ListState,
    pub torrent_infos: HashMap<[u8; 20], TorrentState>,

    /// Used to show and hide the input popup
    show_popup: bool,

    /// If this is Some, a popup will be rendered ontop of the current UI.
    edit_input: Option<Input<'a>>,

    /// The torrent which is currently selected using the UI. Some keybindings
    /// will be aplied to this torrent, such as pause, resume, deletion,
    /// etc.
    active_torrent: Option<[u8; 20]>,
    ctx: Arc<AppCtx>,

    footer: List<'a>,
}

impl<'a> TorrentList<'a> {
    pub fn new(ctx: Arc<AppCtx>) -> Self {
        let k: Line = vec![
            Span::styled("k".to_string(), ctx.style.highlight_fg),
            " move up ".into(),
            Span::styled("j".to_string(), ctx.style.highlight_fg),
            " move down ".into(),
            Span::styled("t".to_string(), ctx.style.highlight_fg),
            " add torrent ".into(),
            Span::styled("p".to_string(), ctx.style.highlight_fg),
            " pause/resume ".into(),
            Span::styled("q".to_string(), ctx.style.highlight_fg),
            " quit".into(),
        ]
        .into();

        let line: ListItem = ListItem::new(k);
        let footer_list: Vec<ListItem> = vec![line];

        let footer = List::new(footer_list)
            .block(Block::default().borders(Borders::ALL).title("Keybindings"));

        Self {
            footer,
            focused: true,
            state: ListState::default(),
            torrent_infos: HashMap::new(),
            show_popup: false,
            edit_input: None,
            active_torrent: None,
            ctx,
        }
    }

    fn next(&mut self) {
        if !self.torrent_infos.is_empty() {
            let i = self.state.selected().map_or(0, |v| {
                if v != self.torrent_infos.len() - 1 {
                    v + 1
                } else {
                    0
                }
            });
            self.state.select(Some(i));
        }
    }

    fn previous(&mut self) {
        if !self.torrent_infos.is_empty() {
            let i = self.state.selected().map_or(0, |v| {
                if v == 0 {
                    self.torrent_infos.len() - 1
                } else {
                    v - 1
                }
            });
            self.state.select(Some(i));
        }
    }

    fn submit_magnet_link(&self, magnet: String) {
        let _ = self.ctx.tx.send(Action::NewTorrent(magnet));
    }
}

impl<'a> Component for TorrentList<'a> {
    fn draw(
        &mut self,
        f: &mut ratatui::prelude::Frame,
        rect: ratatui::prelude::Rect,
    ) {
        let selected = self.state.selected();
        let mut rows: Vec<ListItem> = Vec::new();

        for (i, ctx) in self.torrent_infos.values().enumerate() {
            let mut download_rate = to_human_readable(ctx.download_rate as f64);
            download_rate.push_str("/s");

            let name = Span::from(ctx.name.clone()).bold();

            let status_style = match ctx.status {
                TorrentStatus::ConnectingTrackers
                | TorrentStatus::DownloadingMetainfo => self.ctx.style.base,
                TorrentStatus::Seeding => self.ctx.style.success,
                TorrentStatus::Error => self.ctx.style.error,
                TorrentStatus::Paused => self.ctx.style.warning,
                _ => self.ctx.style.highlight_fg,
            };

            let status_txt: &str = ctx.status.clone().into();
            let mut status_txt =
                vec![Span::styled(status_txt, status_style).bold()];

            if ctx.status == TorrentStatus::Downloading {
                let download_and_rate = format!(
                    " {} - {download_rate}",
                    to_human_readable(ctx.downloaded as f64)
                )
                .into();
                status_txt.push(download_and_rate);
            }

            let s = ctx.stats.seeders.to_string();
            let l = ctx.stats.leechers.to_string();
            let sl = format!("Seeders {s} Leechers {l}").into();

            let mut line_top = Line::from("-".repeat(f.size().width as usize));
            let mut line_bottom = line_top.clone();

            if self.state.selected() == Some(i) {
                line_top.patch_style(self.ctx.style.highlight_fg);
                line_bottom.patch_style(self.ctx.style.highlight_fg);
            }

            let mut items = vec![
                line_top,
                name.into(),
                to_human_readable(ctx.size as f64).into(),
                sl,
                status_txt.into(),
                line_bottom,
            ];

            if Some(i) == selected {
                self.active_torrent = Some(ctx.info_hash);
            }

            if Some(i) != selected && selected > Some(0) {
                items.remove(0);
            }

            rows.push(ListItem::new(items));
        }

        let mut block =
            Block::default().borders(Borders::ALL).title("Torrents");

        if self.focused {
            block = block.set_style(self.ctx.style.highlight_fg);
        }

        let torrent_list = List::new(rows).block(block);

        // Create two chunks, the body, and the footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Max(98), Constraint::Length(3)].as_ref())
            .split(f.size());

        // render list of torrents
        f.render_stateful_widget(torrent_list, chunks[0], &mut self.state);

        // render footer component to help with keybindings
        f.render_widget(self.footer.clone(), chunks[1]);

        if !self.show_popup {
            self.edit_input = None;
        }

        // maybe render popup to add a new torrent using a magnet link
        if let Some(input) = &mut self.edit_input {
            let block =
                Block::default().title("Add new torrent").borders(Borders::ALL);

            input.block = block;

            let area = centered_rect(60, 20, rect);

            f.render_widget(Clear, area);
            input.draw(f, area);
        }
    }

    fn handle_action(
        &mut self,
        action: &crate::action::Action,
    ) -> super::HandleActionResponse {
        let mut response = HandleActionResponse::default();

        // if popup is active, handle actions on it first.
        if let Some(input) = &mut self.edit_input {
            if let Action::Key(k) = action {
                if k.code == KeyCode::Enter {
                    let magnet = input.value.clone();
                    self.submit_magnet_link(magnet);
                    self.show_popup = false;
                } else {
                    input.handle_action(&action);
                }
            }
        }

        match action {
            Action::TorrentState(state) => {
                let t = self
                    .torrent_infos
                    .entry(state.info_hash)
                    .or_insert(TorrentState::default());

                *t = state.clone();
            }
            Action::Key(key) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => self.next(),
                KeyCode::Char('k') | KeyCode::Up => self.previous(),
                KeyCode::Char('t') => {
                    if self.edit_input.is_none() && !self.show_popup {
                        self.show_popup = true;
                        let input = Input::new(self.ctx.style.clone())
                            .focused(true)
                            .mode(Mode::Insert);

                        self.edit_input = Some(input);
                    }
                }
                KeyCode::Char('p') => {
                    if let Some(active_torrent) = self.active_torrent {
                        let _ = self
                            .ctx
                            .tx
                            .send(Action::TogglePause(active_torrent));
                    }
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    if let Some(input) = &mut self.edit_input
                        && self.show_popup
                    {
                        // quit popup if it is in normal mode
                        if input.mode == Mode::Normal {
                            self.show_popup = false;
                            self.edit_input = None;
                        }
                        response = HandleActionResponse::Ignore;
                    }
                }
                _ => {}
            },
            _ => {}
        }

        response
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}
