use std::{fs::create_dir_all, net::SocketAddr, time::Duration};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use bitvec::{bitvec, prelude::Msb0};
use futures::{SinkExt, StreamExt};
use rand::{distributions::Alphanumeric, Rng};
use tokio::{
    fs::OpenOptions, io::AsyncWriteExt, net::{TcpListener, TcpStream}, select, spawn, sync::mpsc, time::interval
};
use tracing::debug;
use vincenzo::{
    daemon::DaemonMsg, disk::{Disk, DiskMsg}, magnet::Magnet, metainfo::Info, peer::{Direction, Peer, PeerMsg}, tcp_wire::{messages::Message, Block, BlockInfo}, torrent::{Torrent, TorrentMsg}, tracker::Tracker
};

// Test that a peer will re-request block_infos after timeout,
// this test will spawn a tracker-less torrent and simulate 2 peers
// communicating with each other, a seeder and a leecher.
//
// The leecher will request one block, the seeder will not answer,
// and then the leecher must send the request again.
#[tokio::test]
async fn peer_request() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .without_time()
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    // tx.send(PeerMsg::RequestBlockInfos(vec![BlockInfo {
    //     index: 0,
    //     begin: 0,
    //     len: 15,
    // }]))
    // .await
    // .unwrap();
    // tokio::time::sleep(Duration::from_secs(5)).await;
    // std::fs::remove_dir_all(download_dir).unwrap();
}
