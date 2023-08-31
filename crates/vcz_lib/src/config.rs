use std::net::SocketAddr;

use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub download_dir: String,
    pub listen: Option<SocketAddr>,
}
