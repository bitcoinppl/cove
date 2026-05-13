#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("full scan request is missing")]
    MissingRequest,

    #[error("stop gap is missing")]
    MissingStopGap,

    #[error("event sender is missing")]
    MissingEvents,

    #[error("scan was cancelled")]
    Cancelled,

    #[error("scan event receiver closed")]
    ChannelClosed,

    #[error("esplora scan failed: {0}")]
    Esplora(#[from] Box<bdk_esplora::esplora_client::Error>),

    #[error("electrum scan failed: {0}")]
    Electrum(#[from] bdk_electrum::electrum_client::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
