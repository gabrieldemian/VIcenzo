use crate::frontend::{FrMsg, TorrentInfo};
use crate::magnet_parser::get_magnet;
use crate::peer::session::ConnectionState;
use crate::tcp_wire::lib::BlockInfo;
use crate::tcp_wire::messages::HandshakeCodec;
use crate::{
    bitfield::Bitfield,
    cli::Args,
    disk::DiskMsg,
    error::Error,
    magnet_parser::get_info_hash,
    metainfo::Info,
    peer::{Direction, Peer, PeerCtx, PeerMsg},
    tracker::{
        event::Event,
        {Tracker, TrackerCtx, TrackerMsg},
    },
};
use bendy::decoding::FromBencode;
use clap::Parser;
use hashbrown::HashMap;
use magnet_url::Magnet;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::{sync::Arc, time::Duration};
use tokio::time::interval;
use tokio::{
    net::{TcpListener, TcpStream},
    select, spawn,
    sync::{mpsc, oneshot, RwLock},
    time::{interval_at, Instant},
};
use tokio_util::codec::Framed;
use tracing::{info, warn};

#[derive(Debug)]
pub enum TorrentMsg {
    /// Message to update the torrent's Bitfield,
    /// Torrent will start with a blank bitfield
    /// because it cannot know it from a magnet link
    /// once a peer send the first bitfield message,
    /// we will update it.
    UpdateBitfield(usize),
    /// Message when one of the peers have downloaded
    /// an entire piece. We send Have messages to peers
    /// that don't have it and update the UI with stats.
    DownloadedPiece(usize),
    PeerConnected([u8; 20], Arc<PeerCtx>),
    DownloadComplete,
    /// When in endgame mode, the first peer that receives this info,
    /// sends this message to send Cancel's to all other peers.
    SendCancel {
        from: [u8; 20],
        block_info: BlockInfo,
    },
    StartEndgame([u8; 20], Vec<BlockInfo>),
    /// When a peer downloads an info piece,
    /// we need to mutate `info_dict` and maybe
    /// generate the entire info.
    /// total, metadata.index, bytes
    DownloadedInfoPiece(u32, u32, Vec<u8>),
    /// When a peer request a piece of the info
    /// index, recipient
    RequestInfoPiece(u32, oneshot::Sender<Option<Vec<u8>>>),
    IncrementDownloaded(u64),
    IncrementUploaded(u64),
    /// When torrent is being gracefully shutdown
    Quit,
}

/// This is the main entity responsible for the high-level management of
/// a torrent download or upload.
#[derive(Debug)]
pub struct Torrent {
    pub ctx: Arc<TorrentCtx>,
    pub tracker_ctx: Arc<TrackerCtx>,
    pub disk_tx: mpsc::Sender<DiskMsg>,
    pub rx: mpsc::Receiver<TorrentMsg>,
    pub peer_ctxs: HashMap<[u8; 20], Arc<PeerCtx>>,
    pub tracker_tx: Option<mpsc::Sender<TrackerMsg>>,
    /// If using a Magnet link, the info will be downloaded in pieces
    /// and those pieces may come in different order,
    /// hence the HashMap (dictionary), and not a vec.
    /// After the dict is complete, it will be decoded into [`info`]
    pub info_pieces: BTreeMap<u32, Vec<u8>>,
    pub have_info: bool,
    /// How many bytes we have uploaded to other peers.
    pub uploaded: u64,
    /// How many bytes we have downloaded from other peers.
    pub downloaded: u64,
    pub fr_tx: mpsc::Sender<FrMsg>,
    pub status: TorrentStatus,
    /// Stats of the current Torrent, returned from tracker on announce requests.
    pub stats: Stats,
    /// The downloaded bytes of the previous second,
    /// used to get the download rate in seconds.
    /// this will be mutated on the frontend event loop.
    pub last_second_downloaded: u64,
    /// The download rate of the torrent, in bytes
    pub download_rate: u64,
    /// The total size of the torrent files, in bytes,
    /// this is a cache of ctx.info.get_size()
    pub size: u64,
    pub name: String,
}

#[derive(Debug)]
pub struct TorrentCtx {
    pub tx: mpsc::Sender<TorrentMsg>,
    pub tracker_tx: RwLock<Option<mpsc::Sender<TrackerMsg>>>,
    pub magnet: Magnet,
    pub info_hash: [u8; 20],
    pub pieces: RwLock<Bitfield>,
    pub info: RwLock<Info>,
}

// Status of the current Torrent, updated at every announce request.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stats {
    pub interval: u32,
    pub leechers: u32,
    pub seeders: u32,
}

impl Torrent {
    pub fn new(disk_tx: mpsc::Sender<DiskMsg>, fr_tx: mpsc::Sender<FrMsg>, magnet: &str) -> Self {
        let magnet = get_magnet(magnet).unwrap_or_else(|_| {
            eprintln!("The magnet link is invalid, try another one");
            std::process::exit(exitcode::USAGE)
        });

        let xt = magnet
            .xt
            .clone()
            .expect("The magnet link does not have a hash");

        let dn = magnet.dn.clone().unwrap_or("Unknown".to_string());

        let pieces = RwLock::new(Bitfield::default());
        let info = RwLock::new(Info::default().name(dn.clone()));
        let info_pieces = BTreeMap::new();
        let tracker_ctx = Arc::new(TrackerCtx::default());

        let info_hash = get_info_hash(&xt);
        let (tx, rx) = mpsc::channel::<TorrentMsg>(300);

        let ctx = Arc::new(TorrentCtx {
            tx: tx.clone(),
            tracker_tx: RwLock::new(None),
            info_hash,
            pieces,
            magnet,
            info,
        });

        Self {
            name: dn,
            size: 0,
            last_second_downloaded: 0,
            download_rate: 0,
            status: TorrentStatus::default(),
            stats: Stats::default(),
            fr_tx,
            uploaded: 0,
            downloaded: 0,
            info_pieces,
            tracker_ctx,
            tracker_tx: None,
            ctx,
            disk_tx,
            rx,
            peer_ctxs: HashMap::new(),
            have_info: false,
        }
    }

    /// Start the Torrent, by sending `connect` and `announce_exchange`
    /// messages to one of the trackers, and returning a list of peers.
    #[tracing::instrument(skip(self), name = "torrent::start")]
    pub async fn start(&mut self, listen: Option<SocketAddr>) -> Result<Vec<Peer>, Error> {
        let mut tracker = Tracker::connect(self.ctx.magnet.tr.clone()).await?;
        let info_hash = self.ctx.clone().info_hash;
        let (res, peers) = tracker.announce_exchange(info_hash, listen).await?;

        self.stats = Stats {
            interval: res.interval,
            seeders: res.seeders,
            leechers: res.leechers,
        };

        info!("new stats {:#?}", self.stats);

        let peers: Vec<Peer> = peers
            .into_iter()
            .map(|addr| {
                let (peer_tx, peer_rx) = mpsc::channel::<PeerMsg>(300);
                let torrent_ctx = self.ctx.clone();
                let tracker_ctx = self.tracker_ctx.clone();
                let disk_tx = self.disk_tx.clone();

                Peer::new(addr, peer_tx, torrent_ctx, peer_rx, disk_tx, tracker_ctx)
            })
            .collect();

        info!("tracker.ctx peer {:?}", self.tracker_ctx.local_peer_addr);

        self.tracker_ctx = tracker.ctx.clone().into();
        self.tracker_tx = Some(tracker.tx.clone());
        let mut ctx_tracker_tx = self.ctx.tracker_tx.write().await;
        *ctx_tracker_tx = Some(tracker.tx.clone());
        drop(ctx_tracker_tx);

        self.tracker_tx = Some(tracker.tx.clone());

        spawn(async move {
            tracker.run().await?;
            Ok::<(), Error>(())
        });

        Ok(peers)
    }

    #[tracing::instrument(skip(self), name = "torrent::start_and_run")]
    pub async fn start_and_run(&mut self, listen: Option<SocketAddr>) -> Result<(), Error> {
        let peers = self.start(listen).await?;

        self.spawn_outbound_peers(peers).await?;
        self.spawn_inbound_peers().await?;
        self.run().await?;

        Ok(())
    }

    /// Spawn an event loop for each peer to listen/send messages.
    pub async fn spawn_outbound_peers(&mut self, peers: Vec<Peer>) -> Result<(), Error> {
        for mut peer in peers {
            info!("outbound peer {:?}", peer.addr);
            peer.session.state.connection = ConnectionState::Connecting;

            // send connections too other peers
            spawn(async move {
                match TcpStream::connect(peer.addr).await {
                    Ok(socket) => {
                        info!("we connected with {:?}", peer.addr);

                        let socket = Framed::new(socket, HandshakeCodec);
                        let socket = peer.start(Direction::Outbound, socket).await?;

                        let r = peer.run(Direction::Outbound, socket).await;

                        if let Err(r) = r {
                            warn!("Peer session stopped due to an error: {}", r);
                        }
                    }
                    Err(e) => {
                        warn!("error with peer: {:?} {e:#?}", peer.addr);
                    }
                }
                // if we are gracefully shutting down, we do nothing with the pending
                // blocks, Rust will drop them when their scope ends naturally.
                // otherwise, we send the blocks back to the torrent
                // so that other peers can download them. In this case, the peer
                // might be shutting down due to an error or this is malicious peer
                // that we wish to end the connection.
                if peer.session.state.connection != ConnectionState::Quitting {
                    peer.free_pending_blocks().await;
                }
                Ok::<(), Error>(())
            });
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn spawn_inbound_peers(&self) -> Result<(), Error> {
        info!("running spawn inbound peers...");
        info!(
            "accepting requests in {:?}",
            self.tracker_ctx.local_peer_addr
        );

        let local_peer_socket = TcpListener::bind(self.tracker_ctx.local_peer_addr).await?;

        let torrent_ctx = Arc::clone(&self.ctx);
        let tracker_ctx = self.tracker_ctx.clone();
        let disk_tx = self.disk_tx.clone();

        // accept connections from other peers
        spawn(async move {
            info!("accepting requests in {local_peer_socket:?}");

            loop {
                if let Ok((socket, addr)) = local_peer_socket.accept().await {
                    let socket = Framed::new(socket, HandshakeCodec);

                    info!("received inbound connection from {addr}");

                    let (peer_tx, peer_rx) = mpsc::channel::<PeerMsg>(300);

                    let mut peer = Peer::new(
                        addr,
                        peer_tx,
                        torrent_ctx.clone(),
                        peer_rx,
                        disk_tx.clone(),
                        tracker_ctx.clone(),
                    );

                    spawn(async move {
                        peer.session.state.connection = ConnectionState::Connecting;
                        let socket = peer.start(Direction::Inbound, socket).await?;

                        let r = peer.run(Direction::Inbound, socket).await;

                        if let Err(r) = r {
                            warn!("Peer session stopped due to an error: {}", r);
                        }

                        // if we are gracefully shutting down, we do nothing with the pending
                        // blocks, Rust will drop them when their scope ends naturally.
                        // otherwise, we send the blocks back to the torrent
                        // so that other peers can download them. In this case, the peer
                        // might be shutting down due to an error or this is malicious peer
                        // that we wish to end the connection.
                        if peer.session.state.connection != ConnectionState::Quitting {
                            peer.free_pending_blocks().await;
                        }

                        Ok::<(), Error>(())
                    });
                }
            }
        });
        Ok(())
    }

    #[tracing::instrument(name = "torrent::run", skip(self))]
    pub async fn run(&mut self) -> Result<(), Error> {
        let tracker_tx = self.tracker_tx.clone().unwrap();

        let mut announce_interval = interval_at(
            Instant::now() + Duration::from_secs(self.stats.interval.max(500).into()),
            Duration::from_secs((self.stats.interval as u64).max(500)),
        );

        let mut frontend_interval = interval(Duration::from_secs(1));

        loop {
            select! {
                Some(msg) = self.rx.recv() => {
                    match msg {
                        TorrentMsg::UpdateBitfield(len) => {
                            // create an empty bitfield with the same
                            // len as the bitfield from the peer
                            let ctx = Arc::clone(&self.ctx);
                            let mut pieces = ctx.pieces.write().await;

                            // only create the bitfield if we don't have one
                            // pieces.len() will start at 0
                            if pieces.len() < len {
                                let inner = vec![0_u8; len];
                                *pieces = Bitfield::from(inner);
                            }
                        }
                        TorrentMsg::DownloadedPiece(piece) => {
                            // send Have messages to peers that dont have our pieces
                            for peer in self.peer_ctxs.values() {
                                let _ = peer.tx.send(PeerMsg::HavePiece(piece)).await;
                            }
                        }
                        TorrentMsg::PeerConnected(id, ctx) => {
                            info!("connected with new peer");
                            self.peer_ctxs.insert(id, ctx);
                        }
                        TorrentMsg::DownloadComplete => {
                            info!("received msg download complete");
                            let (otx, orx) = oneshot::channel();

                            self.status = TorrentStatus::Seeding;

                            let _ = tracker_tx.send(
                                TrackerMsg::Announce {
                                    event: Event::Completed,
                                    info_hash: self.ctx.info_hash,
                                    downloaded: self.downloaded,
                                    uploaded: self.uploaded,
                                    left: 0,
                                    recipient: Some(otx),
                                })
                            .await;

                            if let Ok(Ok(r)) = orx.await {
                                info!("announced completion with success {r:#?}");
                            }

                            // tell all peers that we are not interested,
                            // we wont request blocks from them anymore
                            for peer in self.peer_ctxs.values() {
                                let _ = peer.tx.send(PeerMsg::NotInterested).await;
                            }

                            // announce to tracker that we are stopping
                            if Args::parse().quit_after_complete {
                                let _ = self.ctx.tx.send(TorrentMsg::Quit).await;
                            }
                        }
                        // The peer "from" was the first one to receive the "info".
                        // Send Cancel messages to everyone else.
                        TorrentMsg::SendCancel { from, block_info } => {
                            for (k, peer) in self.peer_ctxs.iter() {
                                if *k == from { continue };
                                let _ = peer.tx.send(PeerMsg::Cancel(block_info.clone())).await;
                            }
                        }
                        TorrentMsg::StartEndgame(_peer_id, block_infos) => {
                            for (_id, peer) in self.peer_ctxs.iter() {
                                let _ = peer.tx.send(PeerMsg::RequestBlockInfos(block_infos.clone())).await;
                            }
                        }
                        TorrentMsg::DownloadedInfoPiece(total, index, bytes) => {
                            if self.status == TorrentStatus::ConnectingTrackers {
                                self.status = TorrentStatus::DownloadingMetainfo;
                            }

                            self.info_pieces.insert(index, bytes);

                            let info_len = self.info_pieces.values().fold(0, |acc, b| {
                                acc + b.len()
                            });

                            let have_all_pieces = info_len as u32 >= total;

                            if have_all_pieces {
                                // info has a valid bencode format
                                let info_bytes = self.info_pieces.values().fold(Vec::new(), |mut acc, b| {
                                    acc.extend_from_slice(b);
                                    acc
                                });
                                let info = Info::from_bencode(&info_bytes).map_err(|_| Error::BencodeError)?;

                                let m_info = self.ctx.magnet.xt.clone().unwrap();

                                let mut hash = sha1_smol::Sha1::new();
                                hash.update(&info_bytes);

                                let hash = hash.digest().bytes();

                                // validate the hash of the downloaded info
                                // against the hash of the magnet link
                                let hash = hex::encode(hash);

                                if hash.to_uppercase() == m_info.to_uppercase() {
                                    self.status = TorrentStatus::Downloading;
                                    info!("the hash of the downloaded info matches the hash of the magnet link");

                                    self.size = info.get_size();
                                    self.have_info = true;

                                    let mut info_l = self.ctx.info.write().await;
                                    info!("new info files {:?}", info.files);
                                    *info_l = info;
                                    drop(info_l);

                                    self.disk_tx.send(DiskMsg::NewTorrent(self.ctx.clone())).await?;
                                } else {
                                    warn!("a peer sent a valid Info, but the hash does not match the hash of the provided magnet link, panicking");
                                    return Err(Error::PieceInvalid);
                                }
                            }
                        }
                        TorrentMsg::RequestInfoPiece(index, recipient) => {
                            let bytes = self.info_pieces.get(&index).cloned();
                            let _ = recipient.send(bytes);
                        }
                        TorrentMsg::IncrementDownloaded(n) => {
                            self.downloaded += n;

                            // check if the torrent download is complete
                            let is_download_complete = self.downloaded >= self.size;
                            info!("yy__downloaded {:?}", self.downloaded);

                            if is_download_complete {
                                info!("download completed!! wont request more blocks");
                                self.ctx.tx.send(TorrentMsg::DownloadComplete).await?;
                            }
                        }
                        TorrentMsg::IncrementUploaded(n) => {
                            self.uploaded += n;
                        }
                        TorrentMsg::Quit => {
                            info!("torrent is quitting");
                            let (otx, orx) = oneshot::channel();
                            let info = self.ctx.info.read().await;
                            let left =
                                if self.downloaded > info.get_size()
                                    { self.downloaded - info.get_size() }
                                else { 0 };

                            let _ = tracker_tx.send(
                                TrackerMsg::Announce {
                                    event: Event::Stopped,
                                    info_hash: self.ctx.info_hash,
                                    downloaded: self.downloaded,
                                    uploaded: self.uploaded,
                                    left,
                                    recipient: Some(otx),
                                })
                            .await;

                            for peer in self.peer_ctxs.values() {
                                let tx = peer.tx.clone();
                                spawn(async move {
                                    let _ = tx.send(PeerMsg::Quit).await;
                                });
                            }

                            orx.await??;

                            return Ok(());
                        }
                    }
                }
                _ = frontend_interval.tick() => {
                    self.download_rate = self.downloaded - self.last_second_downloaded;

                    let torrent_info = TorrentInfo {
                        name: self.name.clone(),
                        size: self.size,
                        downloaded: self.downloaded,
                        uploaded: self.uploaded,
                        stats: self.stats.clone(),
                        status: self.status.clone(),
                        download_rate: self.download_rate,
                    };

                    self.last_second_downloaded = self.downloaded;
                    self.fr_tx.send(FrMsg::Draw(self.ctx.info_hash, torrent_info)).await?;
                }
                // periodically announce to tracker, at the specified interval
                // to update the tracker about the client's stats.
                // let have_info = self.ctx.info_dict;
                _ = announce_interval.tick() => {
                    let info = self.ctx.info.read().await;

                    // we know if the info is downloaded if the piece_length is > 0
                    if info.piece_length > 0 {
                        info!("sending periodic announce, interval {announce_interval:?}");
                        let left = if self.downloaded < info.get_size() { info.get_size() } else { self.downloaded - info.get_size() };

                        let (otx, orx) = oneshot::channel();

                        let _ = tracker_tx.send(
                            TrackerMsg::Announce {
                                event: Event::None,
                                info_hash: self.ctx.info_hash,
                                downloaded: self.downloaded,
                                uploaded: self.uploaded,
                                left,
                                recipient: Some(otx),
                            })
                        .await;

                        let r = orx.await??;
                        info!("new stats {r:#?}");

                        // update our stats, received from the tracker
                        self.stats = r.into();

                        announce_interval = interval(
                            Duration::from_secs(self.stats.interval as u64),
                        );
                    }
                    drop(info);
                }
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum TorrentStatus {
    #[default]
    ConnectingTrackers,
    DownloadingMetainfo,
    Downloading,
    Seeding,
    Error,
}

impl<'a> From<TorrentStatus> for &'a str {
    fn from(val: TorrentStatus) -> Self {
        use TorrentStatus::*;
        match val {
            ConnectingTrackers => "Connecting to trackers",
            DownloadingMetainfo => "Downloading metainfo",
            Downloading => "Downloading",
            Seeding => "Seeding",
            Error => "Error",
        }
    }
}

impl From<TorrentStatus> for String {
    fn from(val: TorrentStatus) -> Self {
        use TorrentStatus::*;
        match val {
            ConnectingTrackers => "Connecting to trackers".to_owned(),
            DownloadingMetainfo => "Downloading metainfo".to_owned(),
            Downloading => "Downloading".to_owned(),
            Seeding => "Seeding".to_owned(),
            Error => "Error".to_owned(),
        }
    }
}

impl From<&str> for TorrentStatus {
    fn from(value: &str) -> Self {
        use TorrentStatus::*;
        match value {
            "Connecting to trackers" => ConnectingTrackers,
            "Downloading metainfo" => DownloadingMetainfo,
            "Downloading" => Downloading,
            "Seeding" => Seeding,
            "Error" | _ => Error,
        }
    }
}
