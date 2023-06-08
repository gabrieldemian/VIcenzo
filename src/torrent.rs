use crate::error::Error;
use crate::magnet_parser::get_info_hash;
use crate::tcp_wire::lib::BlockInfo;
use crate::tcp_wire::messages::HandshakeOld;
use crate::tcp_wire::messages::Message;
use crate::tcp_wire::messages::PeerCodec;
use crate::tracker::tracker::Tracker;
use futures::{SinkExt, StreamExt};
use log::debug;
use log::info;
use magnet_url::Magnet;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::select;
use tokio::spawn;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::time::interval;
use tokio::time::Interval;
use tokio_util::codec::Framed;

#[derive(Debug)]
pub enum TorrentMsg {
    AddMagnet(Magnet),
    ConnectedPeer(SocketAddr),
}

#[derive(Debug)]
pub struct Torrent {
    pub peers: Vec<SocketAddr>,
    pub tx: Sender<TorrentMsg>,
    pub rx: Receiver<TorrentMsg>,
    pub tick_interval: Interval,
}

impl Torrent {
    pub async fn new(tx: Sender<TorrentMsg>, rx: Receiver<TorrentMsg>) -> Self {
        let peers = vec![];

        Self {
            peers,
            tx,
            rx,
            tick_interval: interval(Duration::new(1, 0)),
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        loop {
            self.tick_interval.tick().await;
            debug!("tick torrent");
            if let Ok(msg) = self.rx.try_recv() {
                match msg {
                    TorrentMsg::AddMagnet(link) => {
                        self.add_magnet(link).await.unwrap();
                    }
                    TorrentMsg::ConnectedPeer(addr) => {
                        // this peer has been handshake'd
                        // and is ready to send/receive msgs
                        info!("listening to msgs from {:?}", addr);
                    }
                }
            }
        }
    }

    #[tracing::instrument]
    pub async fn listen_to_peer(
        peer: SocketAddr,
        our_handshake: HandshakeOld,
    ) -> Result<(), Error> {
        let mut tick_timer = interval(Duration::from_secs(1));
        let mut socket = TcpStream::connect(peer).await?;

        // Send Handshake to peer
        socket.write_all(&mut our_handshake.serialize()?).await?;

        // Read Handshake from peer
        let mut handshake_buf = [0u8; 68];
        socket.read_exact(&mut handshake_buf).await?;
        if !HandshakeOld::deserialize(&handshake_buf)?.validate(&our_handshake) {
            return Err(Error::HandshakeInvalid);
        }

        let (mut sink, mut stream) = Framed::new(socket, PeerCodec).split();

        loop {
            select! {
                _ = tick_timer.tick() => {
                    debug!("tick peer {:?}", peer);
                }
                Some(msg) = stream.next() => {
                    let msg = msg?;
                    match msg {
                        Message::Bitfield(bitfield) => {
                            info!("\t received bitfield");
                            let first = bitfield.into_iter().find(|x| *x == 1);
                            if let Some(first) = first {
                                let block = BlockInfo::new().index(first as u32);
                                sink.send(Message::Request(block)).await?;
                            }
                        }
                        Message::Unchoke => {
                            // if we're interested, start sending requests
                            // peer is letting us Request blocks
                            let block = BlockInfo::new().index(2);
                            sink.send(Message::Request(block)).await?;
                        }
                        Message::Choke => {
                            info!("Peer {:?} choked us", peer);
                        }
                        Message::Interested => {
                            info!("Peer {:?} is interested in us", peer);
                            // peer will start to request blocks from us soon
                        }
                        Message::NotInterested => {
                            info!("Peer {:?} is not interested in us", peer);
                            // peer won't request blocks from us anymore
                        }
                        Message::Have { piece_index } => {
                            info!("Peer {:?} has a new piece_index", piece_index);
                        }
                        Message::Piece(block) => {
                            info!("Peer {:?} sent a Piece to us", block);
                        }
                        Message::Cancel(block_info) => {
                            info!("Peer {:?} canceled a block", block_info);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// each connected peer has its own event loop
    #[tracing::instrument]
    pub async fn spawn_peers_tasks(
        peers: Vec<SocketAddr>,
        our_handshake: HandshakeOld,
    ) -> Result<(), Error> {
        for peer in peers {
            let our_handshake = our_handshake.clone();
            debug!("listening to peer...");
            spawn(async move { Self::listen_to_peer(peer, our_handshake) });
        }
        Ok(())
    }

    pub async fn add_magnet(&self, m: Magnet) -> Result<(), Error> {
        debug!("{:#?}", m);
        info!("received add_magnet call");
        let info_hash = get_info_hash(&m.xt.unwrap());
        debug!("info_hash {:?}", info_hash);

        // first, do a `connect` handshake to the tracker
        let tracker = Tracker::connect(m.tr).await?;
        let peer_id = tracker.peer_id;

        // second, do a `announce` handshake to the tracker
        // and get the list of peers for this torrent
        let peers = tracker.announce_exchange(info_hash).await?;

        // listen to events on our peer socket,
        // that we used to announce to trackers.
        // spawn tracker event loop
        let tx = self.tx.clone();
        spawn(async move {
            tracker.run(tx).await;
        });

        info!("sending handshake req to {:?} peers...", peers.len());

        let our_handshake = HandshakeOld::new(info_hash, peer_id);

        // each peer will have its own event loop
        Torrent::spawn_peers_tasks(peers, our_handshake).await?;
        Ok(())
    }
}

//
// choke
// [0, 0, 0, 1, 0]
// [0, 0, 0, 1, 0]
// [0, 0, 0, 1, 0]
//
// unchoke
// [0, 0, 0, 1, 1]
//
// bitfield
// len = 72
// pieces = 71 * 4 (minus the id that is 1)
// 1 bit = 1 piece
// 1 byte = 4 bits or 4 pieces
// 255 means that 4 pieces(bits) in sequence are true(1).
// If they are true, this means the peer has
// that peer available
// [0, 0, 0, 72, 5, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 252]
