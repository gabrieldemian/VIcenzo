use crate::{
    app_style::AppStyle, pages::{home::Home, Page}, tui::Tui
};
use futures::{stream::SplitStream, SinkExt, StreamExt};
use tokio_util::codec::Framed;
use tracing::debug;

use std::{net::SocketAddr, sync::Arc, marker::PhantomData};
use tokio::{
    net::TcpStream, select, spawn, sync::mpsc::{self, unbounded_channel}
};

use vincenzo::{
    daemon_wire::{DaemonCodec, Message}, error::Error
};

use crate::action::Action;

/// The UI runs entirely on the terminal.
/// It will communicate with the [`Daemon`] occasionaly,
/// via TCP messages documented at [`DaemonCodec`].
pub struct App<'a> {
    pub ctx: Arc<AppCtx>,
    pub page: Box<dyn Page>,
    /// If this UI process is running detached from the Daemon,
    /// in it's own process.
    /// If this is the case, we don't want to send a Quit message to
    /// the Daemon when we close the UI.
    pub is_detached: bool,
    pub rx: mpsc::UnboundedReceiver<Action>,
    phantom: PhantomData<&'a i32>,
}

/// Context that is shared between all pages,
/// at the moment, there is only one page [`TorrentList`].
pub struct AppCtx {
    pub tx: mpsc::UnboundedSender<Action>,
    pub style: AppStyle,
}

impl<'a> App<'a> {
    pub fn new() -> Self {
        let (tx, rx) = unbounded_channel();
        let style = AppStyle::new();
        let ctx = Arc::new(AppCtx { tx, style });
        let page = Box::new(Home::new(ctx.clone()));

        App { ctx, rx, page, is_detached: false, phantom: PhantomData }
    }

    /// Listen to the messages sent by the daemon via TCP,
    /// when we receive a message, we send it to ourselves
    /// via mpsc [`Action`]. For example, when we receive
    /// a Draw message from the daemon, we send a Draw message to `run`
    pub async fn listen_daemon(
        app_tx: mpsc::UnboundedSender<Action>,
        mut sink: SplitStream<Framed<TcpStream, DaemonCodec>>,
    ) -> Result<(), Error> {
        debug!("ui listen_daemon");
        loop {
            select! {
                Some(Ok(msg)) = sink.next() => {
                    match msg {
                        Message::TorrentState(Some(state)) => {
                            let _ = app_tx.send(Action::TorrentState(state));
                        }
                        Message::Quit => {
                            debug!("ui Quit");
                            let _ = app_tx.send(Action::Quit);
                            break;
                        }
                        _ => {}
                    }
                }
                else => break,
            }
        }
        Ok(())
    }

    /// Run the UI event loop and connect with the Daemon
    pub async fn run(&mut self, daemon_addr: SocketAddr) -> Result<(), Error> {
        // ratatui terminal
        let mut tui = Tui::new().unwrap().tick_rate(4.0).frame_rate(60.0);
        tui.run().unwrap();
        let tx = self.ctx.tx.clone();

        let fr_tx = self.ctx.tx.clone();

        let socket = TcpStream::connect(daemon_addr).await.unwrap();

        debug!("ui connected to daemon on {:?}", socket.local_addr());

        let socket = Framed::new(socket, DaemonCodec);
        let (mut sink, stream) = socket.split();

        spawn(async move {
            Self::listen_daemon(fr_tx, stream).await.unwrap();
        });

        'outer: loop {
            let e = tui.next().await.unwrap();
            let a = self.page.get_action(e);
            tx.send(a).unwrap();

            while let Ok(action) = self.rx.try_recv() {
                self.page.handle_action(action.clone());

                match action {
                    Action::Quit => {
                        debug!("ui received Quit");
                        self.stop(&mut sink).await?;
                        break 'outer;
                    }
                    Action::Render => {
                        tui.draw(|f| {
                            self.page.draw(f);
                        })?;
                    }
                    Action::NewTorrent(magnet) => {
                        debug!("ui received NewTorrent {magnet}");
                        self.new_torrent(&magnet, &mut sink).await?;
                    }
                    Action::TogglePause(id) => {
                        debug!("ui received TogglePause {id:?}");
                        sink.send(Message::TogglePause(id)).await?;
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    /// Send a NewTorrent message to Daemon, it will answer with a Draw request
    /// with the newly added torrent state.
    async fn new_torrent<T>(
        &mut self,
        magnet: &str,
        sink: &mut T,
    ) -> Result<(), Error>
    where
        T: SinkExt<Message> + Sized + std::marker::Unpin,
    {
        sink.send(Message::NewTorrent(magnet.to_owned()))
            .await
            .map_err(|_| Error::SendErrorTcp)?;
        Ok(())
    }

    async fn stop<T>(&mut self, sink: &mut T) -> Result<(), Error>
    where
        T: SinkExt<Message> + Sized + std::marker::Unpin,
    {
        if !self.is_detached {
            debug!("ui sending quit to daemon");
            sink.send(Message::Quit).await.map_err(|_| Error::SendErrorTcp)?;
        }

        Ok(())
    }
}
