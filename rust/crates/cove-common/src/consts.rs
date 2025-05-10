use std::path::PathBuf;

use eyre::Context as _;
use once_cell::sync::Lazy;

pub static ROOT_DATA_DIR: Lazy<PathBuf> = Lazy::new(data_dir_init);
pub static WALLET_DATA_DIR: Lazy<PathBuf> = Lazy::new(wallet_data_dir_init);
pub static GAP_LIMIT: u8 = 30;

fn data_dir_init() -> PathBuf {
    let dir = dirs::home_dir()
        .expect("failed to get home document directory")
        .join("Library/Application Support/.data");

    init_dir(dir).unwrap()
}

fn wallet_data_dir_init() -> PathBuf {
    let dir = ROOT_DATA_DIR.join("wallets");
    init_dir(dir).unwrap()
}

fn init_dir(dir: PathBuf) -> Result<PathBuf, std::io::Error> {
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .wrap_err_with(|| {
                format!("failed to create wallet data directory at {}", dir.to_string_lossy())
            })
            .unwrap();
    };

    Ok(dir)
}
