use magnet_url::{Magnet, MagnetError};

pub fn get_magnet(str: &str) -> Result<Magnet, MagnetError> {
    let mut m = Magnet::new(str)?;

    // Remove URL encoding of Display Name of torrent
    if let Some(dn) = m.dn.clone() {
        if let Ok(dn) = urlencoding::decode(&dn) {
            m.dn = Some(dn.to_string());
        }
    }

    // Remove URL encoding of trackers URLs
    let tr: Vec<String> =
        m.tr.iter_mut()
            .map(|x| {
                *x = urlencoding::decode(x).unwrap().to_string();
                *x = x.replace("http://", "");
                *x = x.replace("udp://", "");
                // remove any /announce
                if let Some(i) = x.find('/') {
                    *x = x[..i].to_string();
                };
                x.to_owned()
            })
            .collect();
    m.tr = tr;

    Ok(m)
}

/// The infohash from the magnet link needs to be
/// feeded into a SHA1 function, before converting
/// the hex string to a byte vec
pub fn get_info_hash(info: &str) -> [u8; 20] {
    // I dont need to parse to SHA1 because the magnet
    // info_hash is already parsed to SHA1.
    // I would need to do that if I were getting the hash
    // on a .torrent file
    let infohash = hex::decode(info).unwrap();
    let mut x = [0u8; 20];

    x[..20].copy_from_slice(&infohash[..20]);

    x
}
