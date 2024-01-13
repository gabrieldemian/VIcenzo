use std::net::SocketAddr;

use clap::Parser;

use tracing::debug;
use vcz_ui::app::App;
use vincenzo::{config::Config, daemon::Daemon, error::Error};

#[derive(Parser, Debug, Default)]
#[clap(name = "Vincenzo Frontend", author = "Gabriel Lombardo")]
#[command(author, version, about)]
struct Args {
    /// The address that the Daemon is listening on.
    #[clap(short, long)]
    pub daemon_addr: Option<SocketAddr>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Start and run the terminal UI
    let mut app = App::new();

    // is the UI running in a separate binary from the daemon?
    app.is_detached = true;

    let args = Args::parse();
    let config = Config::load().await.unwrap();

    let daemon_addr = args
        .daemon_addr
        .unwrap_or(config.daemon_addr.unwrap_or(Daemon::DEFAULT_LISTENER));

    app.run(daemon_addr).await.unwrap();
    debug!("ui exited run");

    Ok(())
}
