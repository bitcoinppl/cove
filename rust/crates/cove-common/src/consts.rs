use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};

use bitcoin::Amount;
use eyre::Context as _;

static CUSTOM_ROOT_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub static ROOT_DATA_DIR: LazyLock<PathBuf> = LazyLock::new(data_dir_init);
pub static WALLET_DATA_DIR: LazyLock<PathBuf> = LazyLock::new(wallet_data_dir_init);
pub static GAP_LIMIT: u8 = 30;

pub static MIN_SEND_SATS: u64 = 5000;
pub static MIN_SEND_AMOUNT: Amount = Amount::from_sat(MIN_SEND_SATS);

/// Set custom root data directory (must be called before any database access)
/// primarily for Android where we need to use app-specific storage
///
/// # Errors
/// Returns an error if the root data directory has already been initialized
pub fn set_root_data_dir(path: PathBuf) -> Result<(), String> {
    CUSTOM_ROOT_DATA_DIR
        .set(path)
        .map_err(|_| "root data directory already initialized".to_string())
}

fn data_dir_init() -> PathBuf {
    // use custom path if set (Android)
    if let Some(custom_dir) = CUSTOM_ROOT_DATA_DIR.get() {
        return init_dir(custom_dir.clone());
    }

    // iOS: use Library/Application Support
    #[cfg(target_os = "ios")]
    {
        let dir = dirs::home_dir()
            .expect("failed to get home document directory")
            .join("Library/Application Support/.data");
        return init_dir(dir);
    }

    // Android fallback (should use set_root_data_dir instead)
    #[cfg(target_os = "android")]
    {
        panic!("Android must call set_root_data_dir before initializing database");
    }

    // other platforms
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let dir = dirs::home_dir().expect("failed to get home document directory").join(".data");
        init_dir(dir)
    }
}

fn wallet_data_dir_init() -> PathBuf {
    let dir = ROOT_DATA_DIR.join("wallets");
    init_dir(dir)
}

fn init_dir(dir: PathBuf) -> PathBuf {
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .wrap_err_with(|| {
                format!("failed to create wallet data directory at {}", dir.to_string_lossy())
            })
            .unwrap();
    }

    dir
}
