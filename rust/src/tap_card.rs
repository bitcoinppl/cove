use rust_cktap::CkTapCard;
use rust_cktap::apdu::Error as ApduError;
use rust_cktap::commands::{Authentication as _, CkTransport};
use std::fmt::Debug;

// Define error types
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum TransportError {
    #[error("CiborDe: {0}")]
    CiborDe(String),

    #[error("CiborValue: {0}")]
    CiborValue(String),

    #[error("CkTap: erro: {error}, code: {code}")]
    CkTap { error: String, code: u64 },

    #[error("IncorrectSignature: {0}")]
    IncorrectSignature(String),

    #[error("UnknownCardType: {0}")]
    UnknownCardType(String),
}

// Define the callback interface that Swift will implement
#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait TapcardTransportProtocol: Send + Sync + std::fmt::Debug + 'static {
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, TransportError>;
}

// Implement the CkTransport trait for our callback-based transport
#[derive(Debug)]
pub struct TapcardTransport(Box<dyn TapcardTransportProtocol>);

impl CkTransport for TapcardTransport {
    async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, ApduError> {
        let response_bytes = self.0.transmit_apdu(command_apdu).await?;
        Ok(response_bytes)
    }
}

// Convert ApduError type to TransportError for UniFFI
impl From<ApduError> for TransportError {
    fn from(error: ApduError) -> Self {
        match error {
            ApduError::CiborDe(msg) => TransportError::CiborDe(msg),
            ApduError::CiborValue(msg) => TransportError::CiborValue(msg),
            ApduError::CkTap { error, code } => TransportError::CkTap {
                error,
                code: code as u64,
            },
            ApduError::IncorrectSignature(msg) => TransportError::IncorrectSignature(msg),
            ApduError::UnknownCardType(msg) => TransportError::UnknownCardType(msg),
        }
    }
}

impl From<TransportError> for ApduError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::CiborDe(msg) => ApduError::CiborDe(msg),
            TransportError::CiborValue(msg) => ApduError::CiborValue(msg),
            TransportError::CkTap { error, code } => ApduError::CkTap {
                error,
                code: code as usize,
            },
            TransportError::IncorrectSignature(msg) => ApduError::IncorrectSignature(msg),
            TransportError::UnknownCardType(msg) => ApduError::UnknownCardType(msg),
        }
    }
}

// Main interface exposed to Swift
#[derive(Debug, uniffi::Object)]
pub struct TapCardReader(CkTapCard<TapcardTransport>);

#[uniffi::export]
impl TapCardReader {
    #[uniffi::constructor(name = "new")]
    pub async fn new(transport: Box<dyn TapcardTransportProtocol>) -> Result<Self, TransportError> {
        let transport = TapcardTransport(transport);
        let card = transport.to_cktap().await.map_err(TransportError::from)?;
        Ok(Self(card))
    }
}

mod ffi {
    use std::sync::Arc;

    use super::TapCardReader;

    #[uniffi::export]
    pub fn tap_card_is_equal(lhs: Arc<TapCardReader>, rhs: Arc<TapCardReader>) -> bool {
        lhs == rhs
    }
}

impl Eq for TapCardReader {}

impl PartialEq for TapCardReader {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (CkTapCard::SatsCard(a), CkTapCard::SatsCard(b)) => {
                a.pubkey() == a.pubkey()
                    && a.card_nonce() == b.card_nonce()
                    && a.slots == b.slots
                    && a.birth == b.birth
                    && a.auth_delay == b.auth_delay
                    && a.proto == b.proto
                    && a.ver == b.ver
            }
            (CkTapCard::TapSigner(a), CkTapCard::TapSigner(b)) => {
                a.pubkey() == b.pubkey()
                    && a.card_nonce() == b.card_nonce()
                    && a.birth == b.birth
                    && a.path == b.path
                    && a.num_backups == b.num_backups
                    && a.auth_delay == b.auth_delay
                    && a.proto == b.proto
                    && a.ver == b.ver
            }
            (CkTapCard::SatsChip(_), CkTapCard::SatsChip(_)) => unimplemented!(),
            (_, _) => false,
        }
    }
}
