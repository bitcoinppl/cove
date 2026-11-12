use bdk_esplora::esplora_client;

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

    /// Built through `From`, which drops the request URL
    #[error("esplora scan failed: {0}")]
    Esplora(#[source] Box<esplora_client::Error>),

    #[error("electrum scan failed: {0}")]
    Electrum(#[from] bdk_electrum::electrum_client::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<Box<esplora_client::Error>> for Error {
    fn from(error: Box<esplora_client::Error>) -> Self {
        Self::Esplora(Box::new(esplora_error_without_url(*error)))
    }
}

/// Drops the request URL from an esplora error
///
/// A node URL can carry an access token in its path or query, and scan errors reach logs
pub fn esplora_error_without_url(error: esplora_client::Error) -> esplora_client::Error {
    match error {
        esplora_client::Error::Reqwest(error) => {
            esplora_client::Error::Reqwest(error.without_url())
        }
        error => error,
    }
}
