use crossterm::event::KeyEvent;
use vincenzo::torrent::TorrentState;

/// A new component to be rendered on the UI.
/// Used in conjunction with [`Action`]
#[derive(Clone, Copy)]
pub enum Page {
    Home,
}

#[derive(Clone, Debug)]
pub enum Action {
    Tick,
    Key(KeyEvent),
    Render,
    None,
    /// Render another page on the UI
    // ChangePage(Page),
    NewTorrent(String),
    TorrentState(TorrentState),
    TogglePause([u8; 20]),
    Quit,
}
