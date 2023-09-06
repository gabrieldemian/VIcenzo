pub mod error;
pub mod torrent_list;
use clap::Parser;
use futures::{FutureExt, StreamExt};
use hashbrown::HashMap;

use std::{
    io::{self, Stdout},
    sync::Arc,
};
use tokio::{select, spawn, sync::mpsc};

use crossterm::{
    self,
    event::{DisableMouseCapture, EnableMouseCapture, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    style::{Color, Style},
    Terminal,
};

use torrent_list::TorrentList;

use vcz_lib::{
    cli::Args,
    config::Config,
    disk::DiskMsg,
    error::Error,
    torrent::{Torrent, TorrentMsg},
    FrMsg, TorrentState,
};

#[derive(Clone, Debug)]
pub struct AppStyle {
    pub base_style: Style,
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
            base_style: Style::default().fg(Color::Gray),
            highlight_bg: Style::default().bg(Color::LightBlue).fg(Color::DarkGray),
            highlight_fg: Style::default().fg(Color::LightBlue),
            success: Style::default().fg(Color::LightGreen),
            error: Style::default().fg(Color::Red),
            warning: Style::default().fg(Color::Yellow),
        }
    }
}

pub struct Frontend<'a> {
    pub style: AppStyle,
    pub ctx: Arc<FrontendCtx>,
    pub torrent_list: TorrentList<'a>,
    torrent_txs: HashMap<[u8; 20], mpsc::Sender<TorrentMsg>>,
    disk_tx: mpsc::Sender<DiskMsg>,
    terminal: Terminal<CrosstermBackend<Stdout>>,
    config: Config,
}

pub struct FrontendCtx {
    pub fr_tx: mpsc::Sender<FrMsg>,
}

impl<'a> Frontend<'a> {
    pub fn new(fr_tx: mpsc::Sender<FrMsg>, disk_tx: mpsc::Sender<DiskMsg>, config: Config) -> Self {
        let stdout = io::stdout();
        let style = AppStyle::new();
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).unwrap();

        let ctx = Arc::new(FrontendCtx { fr_tx });
        let torrent_list = TorrentList::new(ctx.clone());

        Frontend {
            config,
            terminal,
            torrent_list,
            torrent_txs: HashMap::new(),
            ctx,
            disk_tx,
            style,
        }
    }

    /// Run the UI event loop
    pub async fn run(&mut self, mut fr_rx: mpsc::Receiver<FrMsg>) -> Result<(), Error> {
        let mut reader = EventStream::new();

        // setup terminal
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

        let original_hook = std::panic::take_hook();

        std::panic::set_hook(Box::new(move |panic| {
            Self::reset_terminal();
            original_hook(panic);
        }));

        self.torrent_list.draw(&mut self.terminal).await;

        loop {
            let event = reader.next().fuse();

            select! {
                event = event => {
                    match event {
                        Some(Ok(event)) => {
                            if let crossterm::event::Event::Key(k) = event {
                                self.torrent_list.keybindings(k, &mut self.terminal).await;
                            }
                        }
                        _ => break
                    }
                }
                Some(msg) = fr_rx.recv() => {
                    match msg {
                        FrMsg::Quit => {
                            let _ = self.stop().await;
                            return Ok(());
                        },
                        FrMsg::Draw(info_hash, torrent_info) => {
                            self.torrent_list
                                .torrent_infos
                                .insert(info_hash, torrent_info);

                            self.torrent_list.draw(&mut self.terminal).await;
                        },
                        FrMsg::NewTorrent(magnet) => {
                            self.new_torrent(&magnet).await;
                        }
                        FrMsg::TogglePause(id) => {
                            let tx = self.torrent_txs.get(&id).ok_or(Error::TorrentDoesNotExist)?;
                            tx.send(TorrentMsg::TogglePause).await?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn reset_terminal() {
        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).unwrap();

        disable_raw_mode().unwrap();
        terminal.show_cursor().unwrap();
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .unwrap();
    }

    // Create a Torrent, and then Add it. This will be called when the user
    // adds a torrent using the UI.
    async fn new_torrent(&mut self, magnet: &str) {
        // todo: send message to Daemon to create the torrent
        // the message will return a torrent_tx
        //
        // disk will reply with a FrMsg::AddTorrent and the UI will add the torrent_info
        // only the daemon should know how to create and handle a torrent
        let mut torrent = Torrent::new(self.disk_tx.clone(), self.ctx.fr_tx.clone(), magnet);
        let info_hash = torrent.ctx.info_hash;

        // prevent the user from adding a duplicate torrent,
        // todo: handle this on the UI with a message.
        if self.torrent_txs.get(&info_hash).is_none() {
            self.torrent_txs.insert(info_hash, torrent.ctx.tx.clone());

            let torrent_info_l = TorrentState {
                name: torrent.ctx.info.read().await.name.clone(),
                ..Default::default()
            };

            self.torrent_list
                .torrent_infos
                .insert(info_hash, torrent_info_l);

            let args = Args::parse();
            let mut listen = self.config.listen;

            if args.listen.is_some() {
                listen = args.listen;
            }

            spawn(async move {
                torrent.start_and_run(listen).await.unwrap();
                torrent.disk_tx.send(DiskMsg::Quit).await.unwrap();
            });

            self.torrent_list.draw(&mut self.terminal).await;
        }
    }

    async fn stop(&mut self) {
        Self::reset_terminal();

        // tell all torrents that we are gracefully shutting down,
        // each torrent will kill their peers tasks, and their tracker task
        for (_, tx) in std::mem::take(&mut self.torrent_txs) {
            spawn(async move {
                let _ = tx.send(TorrentMsg::Quit).await;
            });
        }
        let _ = self.disk_tx.send(DiskMsg::Quit).await;
    }
}
