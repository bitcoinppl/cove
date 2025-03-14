use rust_cktap::{
    CkTapCard,
    commands::{Authentication as _, CkTransport as _},
};

use super::{TapcardTransport, TapcardTransportProtocol, TransportError};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum TapSignerReaderError {
    #[error(transparent)]
    TransportError(#[from] TransportError),

    #[error("UnknownCardType: {0}, expected TapSigner")]
    UnknownCardType(String),

    #[error("No command")]
    NoCommand,
}

type Error = TapSignerReaderError;
type Result<T, E = Error> = std::result::Result<T, E>;

// Main interface exposed to Swift
#[derive(Debug, uniffi::Object)]
pub struct TapSignerReader {
    reader: rust_cktap::TapSigner<TapcardTransport>,
    cmd: Option<TapSignerCmd>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TapSignerCmd {
    Setup(SetupCmd),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SetupCmd {
    pub factory_pin: String,
    pub new_pin: String,
    pub chain_code: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TapSignerResponse {
    NoOp,
}

#[uniffi::export]
impl TapSignerReader {
    #[uniffi::constructor(name = "new", default(cmd = None))]
    pub async fn new(
        transport: Box<dyn TapcardTransportProtocol>,
        cmd: Option<TapSignerCmd>,
    ) -> Result<Self> {
        let transport = TapcardTransport(transport);
        let card = transport.to_cktap().await.map_err(TransportError::from)?;

        let card = match card {
            CkTapCard::TapSigner(card) => Ok(card),
            CkTapCard::SatsCard(_) => Err(TapSignerReaderError::UnknownCardType(
                "SatsCard".to_string(),
            )),
            CkTapCard::SatsChip(_) => Err(TapSignerReaderError::UnknownCardType(
                "SatsChip".to_string(),
            )),
        }?;

        Ok(Self { reader: card, cmd })
    }

    #[uniffi::method]
    pub async fn run(&self) -> Result<TapSignerResponse> {
        let cmd = self.cmd.as_ref().ok_or(TapSignerReaderError::NoCommand)?;

        match cmd {
            TapSignerCmd::Setup(cmd) => {
                todo!()
            }
        };
    }
}

// impls

impl Eq for TapSignerReader {}
impl PartialEq for TapSignerReader {
    fn eq(&self, other: &Self) -> bool {
        let (lhs, rhs) = (&self.reader, &other.reader);
        self.cmd == other.cmd
            && lhs.pubkey() == rhs.pubkey()
            && lhs.card_nonce() == rhs.card_nonce()
            && lhs.birth == rhs.birth
            && lhs.path == rhs.path
            && lhs.num_backups == rhs.num_backups
            && lhs.auth_delay == rhs.auth_delay
            && lhs.proto == rhs.proto
            && lhs.ver == rhs.ver
    }
}

mod ffi {
    use super::TapSignerReader;
    use std::sync::Arc;
    #[uniffi::export]
    pub fn tap_card_is_equal(lhs: Arc<TapSignerReader>, rhs: Arc<TapSignerReader>) -> bool {
        lhs == rhs
    }
}
