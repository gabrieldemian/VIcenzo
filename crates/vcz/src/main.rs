use clap::Parser;
use tokio::{runtime::Runtime, spawn};
use tracing::{debug, Level};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt::time::OffsetTime, FmtSubscriber};
use vcz_ui::{action::Action, app::App};
use vincenzo::{
    config::Config, daemon::{Args, Daemon}, error::Error
};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let tmp = std::env::temp_dir();
    let time = std::time::SystemTime::now();
    let _timestamp =
        time.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();

    let _ = std::fs::remove_file("/tmp/vcz.log");

    let file_appender =
        RollingFileAppender::new(Rotation::NEVER, tmp, format!("vcz.log"));
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .with_writer(non_blocking)
        .with_timer(OffsetTime::new(
            time::UtcOffset::current_local_offset()
                .unwrap_or(time::UtcOffset::UTC),
            time::format_description::parse(
                "[year]-[month]-[day] [hour]:[minute]:[second]",
            )
            .unwrap(),
        ))
        .with_ansi(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    let args = Args::parse();
    let config = Config::load().await.unwrap();

    let download_dir = args.download_dir.unwrap_or(config.download_dir.clone());
    let daemon_addr = args
        .daemon_addr
        .unwrap_or(config.daemon_addr.unwrap_or(Daemon::DEFAULT_LISTENER));

    let mut daemon = Daemon::new(download_dir);
    daemon.config.listen = daemon_addr;

    let rt = Runtime::new().unwrap();
    let handle = std::thread::spawn(move || {
        rt.block_on(async {
            daemon.run().await.unwrap();
            debug!("daemon exited run");
        });
    });

    // Start and run the terminal UI
    let mut fr = App::new();
    let fr_tx = fr.ctx.tx.clone();

    let args = Args::parse();

    // If the user passed a magnet through the CLI,
    // start this torrent immediately
    if let Some(magnet) = args.magnet {
        fr_tx.send(Action::NewTorrent(magnet)).unwrap();
    }

    spawn(async move {
        handle.join().unwrap();
    });

    fr.run(daemon_addr).await.unwrap();
    debug!("ui exited run");

    Ok(())
}
