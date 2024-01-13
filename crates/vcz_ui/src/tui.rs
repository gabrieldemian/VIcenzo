use std::{
    ops::{Deref, DerefMut}, time::Duration
};

use crate::error::Error;
use crossterm::{
    cursor, event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event as CrosstermEvent, KeyEvent, KeyEventKind, MouseEvent
    }, terminal::{EnterAlternateScreen, LeaveAlternateScreen}
};
use futures::{FutureExt, StreamExt};
use ratatui::backend::CrosstermBackend as Backend;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender}, task::JoinHandle
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub enum Event {
    Init,
    Quit,
    Error,
    Closed,
    Tick,
    Render,
    FocusGained,
    FocusLost,
    Paste(String),
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
}

pub struct Tui {
    pub terminal: ratatui::Terminal<Backend<std::io::Stderr>>,
    pub task: JoinHandle<()>,
    pub cancellation_token: CancellationToken,
    pub event_rx: UnboundedReceiver<Event>,
    pub event_tx: UnboundedSender<Event>,
    pub frame_rate: f64,
    pub tick_rate: f64,
    pub mouse: bool,
    pub paste: bool,
}

impl Tui {
    pub fn new() -> Result<Self, Error> {
        let tick_rate = 4.0;
        let frame_rate = 60.0;
        let terminal =
            ratatui::Terminal::new(Backend::new(std::io::stderr())).unwrap();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let cancellation_token = CancellationToken::new();
        let task = tokio::spawn(async {});
        let mouse = false;
        let paste = false;

        Ok(Self {
            terminal,
            task,
            cancellation_token,
            event_rx,
            event_tx,
            frame_rate,
            tick_rate,
            mouse,
            paste,
        })
    }

    pub fn tick_rate(mut self, tick_rate: f64) -> Self {
        self.tick_rate = tick_rate;
        self
    }

    pub fn frame_rate(mut self, frame_rate: f64) -> Self {
        self.frame_rate = frame_rate;
        self
    }

    #[allow(dead_code)]
    pub fn mouse(mut self, mouse: bool) -> Self {
        self.mouse = mouse;
        self
    }

    #[allow(dead_code)]
    pub fn paste(mut self, paste: bool) -> Self {
        self.paste = paste;
        self
    }

    fn start(&mut self) {
        let tick_delay = Duration::from_secs_f64(1.0 / self.tick_rate);
        let render_delay = Duration::from_secs_f64(1.0 / self.frame_rate);

        self.cancel();
        self.cancellation_token = CancellationToken::new();

        let _cancellation_token = self.cancellation_token.clone();
        let _event_tx = self.event_tx.clone();

        self.task = tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_delay);
            let mut render_interval = tokio::time::interval(render_delay);

            _event_tx.send(Event::Init).unwrap();

            loop {
                let tick_delay = tick_interval.tick();
                let render_delay = render_interval.tick();
                let crossterm_event = reader.next().fuse();

                tokio::select! {
                    _ = _cancellation_token.cancelled() => {
                        break;
                    }
                    maybe_event = crossterm_event => {
                      match maybe_event {
                        Some(Ok(evt)) => {
                          match evt {
                            CrosstermEvent::Key(key) => {
                              if key.kind == KeyEventKind::Press {
                                  _event_tx.send(Event::Key(key)).unwrap();
                              }
                            },
                            CrosstermEvent::Mouse(mouse) => {
                                _event_tx.send(Event::Mouse(mouse)).unwrap();
                            },
                            CrosstermEvent::Resize(x, y) => {
                                _event_tx.send(Event::Resize(x, y)).unwrap();
                            },
                            CrosstermEvent::FocusLost => {
                                _event_tx.send(Event::FocusLost).unwrap();
                            },
                            CrosstermEvent::FocusGained => {
                                _event_tx.send(Event::FocusGained).unwrap();
                            },
                            CrosstermEvent::Paste(s) => {
                                _event_tx.send(Event::Paste(s)).unwrap();
                            },
                          }
                        }
                        Some(Err(_)) => {
                            _event_tx.send(Event::Error).unwrap();
                        }
                        None => {},
                      }
                  },
                  _ = tick_delay => {
                      _event_tx.send(Event::Tick).unwrap();
                  },
                  _ = render_delay => {
                      _event_tx.send(Event::Render).unwrap();
                  },
                }
            }
        });
    }

    pub fn stop(&self) -> Result<(), Error> {
        let mut counter = 0;
        self.cancel();

        while !self.task.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
            counter += 1;
            if counter > 50 {
                self.task.abort();
            }
            if counter > 100 {
                // log::error!("Failed to abort task in 100 milliseconds for
                // unknown reason");
                break;
            }
        }

        Ok(())
    }

    pub fn run(&mut self) -> Result<(), Error> {
        crossterm::terminal::enable_raw_mode().unwrap();
        crossterm::execute!(
            std::io::stderr(),
            EnterAlternateScreen,
            cursor::Hide
        )
        .unwrap();
        if self.mouse {
            crossterm::execute!(std::io::stderr(), EnableMouseCapture).unwrap();
        }
        if self.paste {
            crossterm::execute!(std::io::stderr(), EnableBracketedPaste)
                .unwrap();
        }
        self.start();

        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            crossterm::execute!(
                std::io::stderr(),
                LeaveAlternateScreen,
                cursor::Show
            )
            .unwrap();
            crossterm::terminal::disable_raw_mode().unwrap();
            original_hook(panic);
        }));

        Ok(())
    }

    pub fn exit(&mut self) -> Result<(), Error> {
        self.stop()?;
        if crossterm::terminal::is_raw_mode_enabled().unwrap() {
            self.flush().unwrap();
            if self.paste {
                crossterm::execute!(std::io::stderr(), DisableBracketedPaste)
                    .unwrap();
            }
            if self.mouse {
                crossterm::execute!(std::io::stderr(), DisableMouseCapture)
                    .unwrap();
            }
            crossterm::execute!(
                std::io::stderr(),
                LeaveAlternateScreen,
                cursor::Show
            )
            .unwrap();
            crossterm::terminal::disable_raw_mode().unwrap();
        }
        Ok(())
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    #[allow(dead_code)]
    pub fn suspend(&mut self) -> Result<(), Error> {
        self.exit()?;
        #[cfg(not(windows))]
        signal_hook::low_level::raise(signal_hook::consts::signal::SIGTSTP)
            .unwrap();
        Ok(())
    }

    #[allow(dead_code)]
    pub fn resume(&mut self) -> Result<(), Error> {
        self.run()?;
        Ok(())
    }

    pub async fn next(&mut self) -> Result<Event, Error> {
        Ok(self.event_rx.recv().await.unwrap())
    }
}

impl Deref for Tui {
    type Target = ratatui::Terminal<Backend<std::io::Stderr>>;

    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        self.exit().unwrap();
    }
}
