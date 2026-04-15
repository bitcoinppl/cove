use std::sync::Arc;

use nid::Nanoid;
use parking_lot::Mutex as SyncMutex;
use rust_cktap::{
    CkTapCard,
    commands::{Authentication, CkTransport as _, Wait as _},
};
use tokio::sync::Mutex;
use tracing::debug;

use super::{TapcardTransport, TapcardTransportProtocol, TransportError};

#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum SatsCardReaderError {
    #[error(transparent)]
    SatsCardError(#[from] TransportError),

    #[error("UnknownCardType: {0}, expected SatsCard")]
    UnknownCardType(String),

    #[error("No command")]
    NoCommand,

    #[error("No active slot address")]
    NoActiveAddress,

    #[error("All slots have been used")]
    AllSlotsUsed,

    #[error("Unknown error: {0}")]
    Unknown(String),
}

type Error = SatsCardReaderError;
type Result<T, E = Error> = std::result::Result<T, E>;

/// Current slot status returned from a SATSCARD
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct SatsCardSlotStatus {
    pub active_slot: u8,
    pub num_slots: u8,
    pub address: Option<String>,
    pub is_sealed: bool,
}

/// Commands that can be issued to a SATSCARD
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum SatsCardCmd {
    Status,
    Unseal { slot: u8, cvc: String },
}

/// Responses from SATSCARD commands
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SatsCardResponse {
    Status(SatsCardSlotStatus),
    Unseal(SatsCardUnsealResult),
}

/// Result of unsealing a SATSCARD slot, contains the private key for sweeping
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct SatsCardUnsealResult {
    pub slot: u8,
    pub private_key: Vec<u8>,
    pub pubkey: Vec<u8>,
    pub master_pubkey: Vec<u8>,
    pub chain_code: Vec<u8>,
}

#[derive(Debug, uniffi::Object)]
pub struct SatsCardReader {
    id: String,
    reader: Mutex<rust_cktap::SatsCard<TapcardTransport>>,
    transport: TapcardTransport,
    slot_status: SyncMutex<Option<SatsCardSlotStatus>>,
}

impl SatsCardReader {
    async fn new(transport: Box<dyn TapcardTransportProtocol>) -> Result<Self> {
        let transport = TapcardTransport(Arc::new(transport));
        let card = transport.clone().to_cktap().await.map_err(TransportError::from)?;

        debug!("sats_card_from_status: {:?}", card);

        let card = match card {
            CkTapCard::SatsCard(card) => Ok(card),
            CkTapCard::TapSigner(_) => {
                Err(SatsCardReaderError::UnknownCardType("TapSigner".to_string()))
            }
            CkTapCard::SatsChip(_) => {
                Err(SatsCardReaderError::UnknownCardType("SatsChip".to_string()))
            }
        }?;

        let id: Nanoid = Nanoid::new();

        let status = SatsCardSlotStatus {
            active_slot: card.slots.0,
            num_slots: card.slots.1,
            address: card.addr.clone(),
            is_sealed: card.addr.is_some(),
        };

        let me = Self {
            id: id.to_string(),
            reader: Mutex::new(card),
            transport,
            slot_status: SyncMutex::new(Some(status)),
        };

        me.wait_if_needed().await?;

        Ok(me)
    }
}

#[uniffi::export]
impl SatsCardReader {
    #[uniffi::method]
    pub async fn run(&self, cmd: SatsCardCmd) -> Result<SatsCardResponse> {
        match cmd {
            SatsCardCmd::Status => {
                let status = self.read_slot_status().await?;
                Ok(SatsCardResponse::Status(status))
            }
            SatsCardCmd::Unseal { slot, cvc } => {
                let result = self.unseal(slot, &cvc).await?;
                Ok(SatsCardResponse::Unseal(result))
            }
        }
    }

    /// Get the cached slot status without NFC communication
    pub fn cached_status(&self) -> Option<SatsCardSlotStatus> {
        self.slot_status.lock().clone()
    }
}

impl SatsCardReader {
    async fn wait_if_needed(&self) -> Result<(), Error> {
        let mut auth_delay = self.reader.lock().await.auth_delay;

        while let Some(delay) = auth_delay {
            let message = format!("Too many attempts, waiting for {delay} seconds...");
            self.transport.set_message(message);

            self.reader.lock().await.wait(None).await.map_err(TransportError::from)?;
            auth_delay = self.reader.lock().await.auth_delay;
        }

        self.reader.lock().await.set_auth_delay(None);
        Ok(())
    }

    async fn read_slot_status(&self) -> Result<SatsCardSlotStatus> {
        let reader = self.reader.lock().await;

        let status = SatsCardSlotStatus {
            active_slot: reader.slots.0,
            num_slots: reader.slots.1,
            address: reader.addr.clone(),
            is_sealed: reader.addr.is_some(),
        };

        *self.slot_status.lock() = Some(status.clone());

        Ok(status)
    }

    async fn unseal(&self, slot: u8, cvc: &str) -> Result<SatsCardUnsealResult> {
        let mut reader = self.reader.lock().await;

        if reader.slots.0 >= reader.slots.1 {
            return Err(Error::AllSlotsUsed);
        }

        let unseal_response = reader.unseal(slot, cvc).await.map_err(TransportError::from)?;

        Ok(SatsCardUnsealResult {
            slot: unseal_response.slot,
            private_key: unseal_response.privkey.to_vec(),
            pubkey: unseal_response.pubkey.to_vec(),
            master_pubkey: unseal_response.master_pk.to_vec(),
            chain_code: unseal_response.chain_code.to_vec(),
        })
    }
}

/// Create a SatsCardReader instance for FFI callers
/// UniFFI's Kotlin bindings do not support async primary constructors
#[uniffi::export]
pub async fn create_sats_card_reader(
    transport: Box<dyn TapcardTransportProtocol>,
) -> Result<Arc<SatsCardReader>, SatsCardReaderError> {
    let reader = SatsCardReader::new(transport).await?;
    Ok(Arc::new(reader))
}

impl std::hash::Hash for SatsCardReader {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.slot_status.lock().hash(state);
    }
}

impl Eq for SatsCardReader {}
impl PartialEq for SatsCardReader {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
