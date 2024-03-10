use bitvec::{bitvec, prelude::Msb0};
use rand::{distributions::Alphanumeric, Rng};
use std::{net::SocketAddr, path::Path, sync::Arc};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, spawn};
use vincenzo::{
    daemon::{Daemon, DaemonCtx, DaemonMsg}, magnet::Magnet, metainfo::Info, torrent::TorrentCtx, tracker::Tracker
};

/// The data returned when a fake peer is downloading/uploading a Torrent
#[derive(Clone, Debug)]
pub struct TorrentInfo {
    pub daemon_ctx: Arc<DaemonCtx>,
    pub torrent_ctx: Arc<TorrentCtx>,
}


/// Setup necessary boilerplate to test a Torrent,
/// create download folder, initialize structs,
/// but do not spawn the daemon.
fn setup_torrent() -> (Daemon, Magnet, Info) {
    let original_hook = std::panic::take_hook();
    let mut rng = rand::thread_rng();

    // name of the torrent file
    let name: String =
        (0..20).map(|_| rng.sample(Alphanumeric) as char).collect();
    let download_dir: String = "/tmp".into();
    let info_hash = [9u8; 20];
    let info_hash_str: String = info_hash.iter().map(|_| "9").collect();
    let local_peer_id = Tracker::gen_peer_id();

    let download_dir_2: String = download_dir.clone();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = std::fs::remove_dir_all(&download_dir_2);
        original_hook(panic);
    }));

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(format!("{download_dir}/{name}"))
        .await
        .unwrap();

    let bytes = [3u8; 30_usize];
    file.write_all(&bytes).await.unwrap();

    let magnet = format!("magnet:?xt=urn:btih:{info_hash_str}&amp;dn={name}&amp;tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969%2Fannounce");
    let info = Info {
        file_length: Some(30),
        name,
        piece_length: 15,
        pieces: vec![0; 40],
        files: None,
    };

    let magnet = Magnet::new(&magnet).unwrap();
    let daemon = Daemon::new(download_dir.clone());

    (daemon, magnet, info)
}

/// Create a seeder node for a torrent with a random name
pub async fn create_seeder() -> TorrentInfo {
    let (daemon, magnet, info) = setup_torrent();
    let daemon_ctx = daemon.ctx.clone();
    let info_hash = magnet.parse_xt();

    spawn(async move {
        daemon.run().await.unwrap();
    });

    let seeder_addr: SocketAddr = "127.0.0.1:3333".parse().unwrap();
    let peers = vec![seeder_addr];

    daemon_ctx
        .tx
        .send(DaemonMsg::AddTorrentWithPeers(magnet, peers))
        .await
        .unwrap();

    let mut ctxs = daemon_ctx.torrent_ctxs.write().await;
    let torrent_ctx = ctxs.get_mut(&info_hash).unwrap();
    let mut daemon_info = torrent_ctx.info.write().await;

    // this is a seeder, so it already has the info.
    let mut have_info = torrent_ctx.have_info.write().await;
    *have_info = true;
    drop(have_info);

    // populate the bitfield with zeroes
    let mut torrent_bitfield = torrent_ctx.bitfield.write().await;
    *torrent_bitfield = bitvec![u8, Msb0; 0; info.pieces() as usize];
    drop(torrent_bitfield);

    // pretend that daemon already has the info of the torrent
    *daemon_info = info.clone();
    drop(daemon_info);

    TorrentInfo {
        daemon_ctx,
        torrent_ctx: *torrent_ctx,
    }
}
