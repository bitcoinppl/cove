use std::hash::Hasher;
use std::sync::Arc;

use bitcoin::{Address, CompressedPublicKey, KnownHrp, secp256k1::PublicKey};
use nid::Nanoid;
use parking_lot::RwLock;
use rust_cktap::{
    CkTapCard,
    commands::{Authentication as _, CkTransport as _, Read as _, Wait as _},
};
use tokio::sync::Mutex;
use tracing::debug;
use zeroize::Zeroizing;

use crate::database::Database;
use crate::network::Network;

use super::{CkTapError, TapcardTransport, TapcardTransportProtocol, TransportError};

// MARK: - Errors

#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum SatsCardReaderError {
    #[error(transparent)]
    SatsCardError(#[from] TransportError),

    #[error("UnknownCardType: {0}, expected SatsCard")]
    UnknownCardType(String),

    #[error("No command")]
    NoCommand,

    #[error("Invalid CVC length, must be 6 digits, found {0}")]
    InvalidCvcLength(u8),

    #[error("CVC must be numeric only")]
    NonNumericCvc,

    #[error("Slot {slot} is out of range, card has {num_slots} slots")]
    SlotOutOfRange { slot: u8, num_slots: u8 },

    #[error("Card did not report an address for the active slot")]
    NoAddress,

    #[error("Slot pubkey could not be parsed")]
    InvalidPubkey,

    #[error("Card-reported address does not match the slot pubkey")]
    AddressMismatch { card_reported: String, derived: String },

    #[error("Card-reported address does not match the URL suffix scanned")]
    SuffixMismatch { card_reported: String, expected_suffix: String },

    #[error("Unknown error: {0}")]
    Unknown(String),
}

type Error = SatsCardReaderError;
type Result<T, E = Error> = std::result::Result<T, E>;

// MARK: - Commands and Responses

#[derive(Debug, Clone, uniffi::Enum)]
pub enum SatsCardCmd {
    /// Get the current status of the card (no CVC needed)
    Status,

    /// Unseal the active slot to reveal the private key.
    ///
    /// On success the FFI caller receives only an opaque
    /// [`SatsCardSweepSession`] handle — raw key bytes never cross the
    /// boundary.
    Unseal { cvc: String },

    /// Get info about a specific slot (status only — never returns key material
    /// across FFI even when a CVC is supplied internally).
    Dump { slot: u8 },
}

#[derive(Debug, uniffi::Enum)]
pub enum SatsCardResponse {
    Status(SatsCardStatus),
    /// Opaque handle. The unsealed key material is owned Rust-side and
    /// zeroised on drop (see [`SatsCardSweepSession`]).
    Unseal(Arc<SatsCardSweepSession>),
    Dump(SatsCardSlotDump),
}

// MARK: - Slot and Status types

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct SatsCardStatus {
    /// Active slot number (0-indexed)
    pub active_slot: u8,
    /// Total number of slots on the card
    pub num_slots: u8,
    /// Card-reported address for the active slot — UNVERIFIED.
    ///
    /// Treat this as a display hint only. Call
    /// [`SatsCardReader::verified_address`] before showing balance or
    /// allowing a sweep.
    pub address: Option<String>,
    /// Protocol version
    pub proto: u32,
    /// Firmware version
    pub ver: String,
    /// Auth delay remaining (rate limit after bad CVC)
    pub auth_delay: Option<u32>,
    /// The network the card is on
    pub network: Network,
}

/// Internal-only payload of an `unseal` APDU.
///
/// **Not** a `uniffi::Record`. Crate-private. Constructed once inside
/// [`SatsCardReader::unseal`] and immediately consumed by
/// [`SatsCardSweepSession::from_unseal`]; raw key bytes never escape Rust.
pub(crate) struct SatsCardUnsealData {
    pub slot: u8,
    pub privkey: Zeroizing<[u8; 32]>,
    pub pubkey: PublicKey,
    pub master_pk: Zeroizing<[u8; 32]>,
    pub chain_code: Zeroizing<[u8; 32]>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct SatsCardSlotDump {
    pub slot: u8,
    /// 33-byte compressed slot pubkey (safe to expose).
    pub pubkey: Vec<u8>,
    pub used: Option<bool>,
    pub sealed: Option<bool>,
    pub address: Option<String>,
}

// MARK: - SatsCardSweepSession (opaque, key material lives Rust-side only)

/// Owns the unsealed slot's key material for the duration of one sweep.
///
/// `privkey`, `master_pk`, and `chain_code` are wrapped in
/// [`Zeroizing`], so the underlying buffers are scrubbed when the
/// session drops. The FFI surface only exposes non-sensitive accessors
/// (`slot`, `address`). Phase 3 will add `balance()` and
/// `build_and_broadcast()` on this type — both consuming the session so
/// it can never be reused.
#[derive(uniffi::Object)]
pub struct SatsCardSweepSession {
    id: String,
    slot: u8,
    privkey: Zeroizing<[u8; 32]>,
    pubkey: PublicKey,
    #[allow(dead_code)] // consumed in Phase 3 sweep flow
    master_pk: Zeroizing<[u8; 32]>,
    #[allow(dead_code)] // consumed in Phase 3 sweep flow
    chain_code: Zeroizing<[u8; 32]>,
    address: String,
    network: Network,
}

impl std::fmt::Debug for SatsCardSweepSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit any field that could lead a debug log to
        // print key material.
        f.debug_struct("SatsCardSweepSession")
            .field("id", &self.id)
            .field("slot", &self.slot)
            .field("address", &self.address)
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

impl SatsCardSweepSession {
    fn from_unseal(data: SatsCardUnsealData, address: String, network: Network) -> Self {
        Self {
            id: Nanoid::<8>::new().to_string(),
            slot: data.slot,
            privkey: data.privkey,
            pubkey: data.pubkey,
            master_pk: data.master_pk,
            chain_code: data.chain_code,
            address,
            network,
        }
    }

    /// Crate-internal accessor for Phase 3 sweep code. Never `pub`.
    #[allow(dead_code)]
    pub(crate) fn privkey_bytes(&self) -> &[u8; 32] {
        &self.privkey
    }
}

#[uniffi::export]
impl SatsCardSweepSession {
    pub fn slot(&self) -> u8 {
        self.slot
    }

    /// The verified P2WPKH address for the unsealed slot.
    pub fn address(&self) -> String {
        self.address.clone()
    }

    /// 33-byte compressed slot pubkey (safe to expose).
    pub fn pubkey_bytes(&self) -> Vec<u8> {
        self.pubkey.serialize().to_vec()
    }

    pub fn network(&self) -> Network {
        self.network
    }
}

// MARK: - SatsCardReader

#[derive(Debug, uniffi::Object)]
pub struct SatsCardReader {
    id: String,
    reader: Mutex<rust_cktap::SatsCard<TapcardTransport>>,
    cmd: RwLock<Option<SatsCardCmd>>,
    transport: TapcardTransport,
    network: Network,
    /// 8-character suffix from the scanned `getsatscard.com/start#…r=…` URL,
    /// used by [`Self::verified_address`] as a sanity check.
    expected_url_suffix: Option<String>,
}

impl SatsCardReader {
    async fn new(
        transport: Box<dyn TapcardTransportProtocol>,
        cmd: Option<SatsCardCmd>,
        expected_url_suffix: Option<String>,
    ) -> Result<Self> {
        let transport = TapcardTransport(Arc::new(transport));
        let card = transport.clone().to_cktap().await.map_err(TransportError::from)?;

        let card_type = match &card {
            CkTapCard::SatsCard(_) => "SatsCard",
            CkTapCard::TapSigner(_) => "TapSigner",
            CkTapCard::SatsChip(_) => "SatsChip",
        };
        debug!("sats_card_reader: detected card type {card_type}");

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
        let network = Database::global().global_config.selected_network();

        let me = Self {
            id: id.to_string(),
            reader: Mutex::new(card),
            transport,
            cmd: RwLock::new(cmd),
            network,
            expected_url_suffix,
        };

        me.wait_if_needed().await?;

        Ok(me)
    }
}

#[uniffi::export]
impl SatsCardReader {
    #[uniffi::method]
    pub async fn run(&self) -> Result<SatsCardResponse> {
        let cmd = self.cmd.write().take().ok_or(SatsCardReaderError::NoCommand)?;

        match cmd {
            SatsCardCmd::Status => {
                let status = self.status().await?;
                Ok(SatsCardResponse::Status(status))
            }

            SatsCardCmd::Unseal { cvc } => {
                let session = self.unseal(&cvc).await?;
                Ok(SatsCardResponse::Unseal(session))
            }

            SatsCardCmd::Dump { slot } => {
                let data = self.dump(slot).await?;
                Ok(SatsCardResponse::Dump(data))
            }
        }
    }

    /// Get the current status of the SatsCard.
    ///
    /// `address` on the returned status is the **card-reported, unverified**
    /// string. Call [`Self::verified_address`] before showing balance or
    /// allowing a sweep.
    pub async fn status(&self) -> Result<SatsCardStatus> {
        let reader = self.reader.lock().await;

        Ok(SatsCardStatus {
            active_slot: reader.slots.0,
            num_slots: reader.slots.1,
            address: reader.addr.clone(),
            proto: reader.proto as u32,
            ver: reader.ver.clone(),
            auth_delay: reader.auth_delay.map(|d| d as u32),
            network: self.network,
        })
    }

    /// Verify that the active slot's address really belongs to the slot
    /// pubkey reported via the `read` APDU, and that the URL suffix scanned
    /// (when present) matches the card-reported address.
    ///
    /// `read()` performs an internal signature check against the card's
    /// master pubkey. We re-derive the P2WPKH address from the verified
    /// slot pubkey and require it to match `reader.addr`. The optional
    /// 8-character URL suffix is then a cheap second-source sanity check.
    ///
    /// The UI must call this and observe `Ok` before displaying balance
    /// or enabling the sweep CTA.
    pub async fn verified_address(&self) -> Result<String> {
        let mut reader = self.reader.lock().await;

        let claimed = reader.addr.clone().ok_or(SatsCardReaderError::NoAddress)?;

        let read_resp = reader.read(None).await.map_err(TransportError::from)?;

        let slot_pk = CompressedPublicKey::from_slice(&read_resp.pubkey)
            .map_err(|_| SatsCardReaderError::InvalidPubkey)?;

        let bnetwork: bitcoin::Network = self.network.into();
        let derived = Address::p2wpkh(&slot_pk, KnownHrp::from(bnetwork)).to_string();

        if derived != claimed {
            return Err(SatsCardReaderError::AddressMismatch { card_reported: claimed, derived });
        }

        if let Some(suffix) = self.expected_url_suffix.as_deref()
            && !claimed.ends_with(suffix)
        {
            return Err(SatsCardReaderError::SuffixMismatch {
                card_reported: claimed,
                expected_suffix: suffix.to_string(),
            });
        }

        Ok(claimed)
    }
}

impl SatsCardReader {
    async fn wait_if_needed(&self) -> Result<(), Error> {
        loop {
            let delay = {
                let reader = self.reader.lock().await;
                reader.auth_delay
            };

            let Some(delay) = delay else { break };

            self.transport
                .set_message(format!("Too many CVC attempts, waiting for {delay} seconds..."));

            {
                let mut reader = self.reader.lock().await;
                reader.wait(None).await.map_err(TransportError::from)?;
            }
        }

        {
            let mut reader = self.reader.lock().await;
            reader.set_auth_delay(None);
        }

        Ok(())
    }

    async fn unseal(&self, cvc: &str) -> Result<Arc<SatsCardSweepSession>> {
        validate_cvc(cvc)?;

        // Verify the slot address before consuming the unseal — if the card
        // is lying about its address, the sweep would broadcast funds the
        // user can't recover.
        let address = self.verified_address().await?;

        let mut reader = self.reader.lock().await;
        let active_slot = reader.slots.0;

        let response = reader.unseal(active_slot, cvc).await.map_err(TransportError::from)?;

        let pubkey = PublicKey::from_slice(&response.pubkey)
            .map_err(|_| SatsCardReaderError::InvalidPubkey)?;

        let unseal_data = SatsCardUnsealData {
            slot: response.slot,
            privkey: clone_into_zeroizing_32(&response.privkey)?,
            pubkey,
            master_pk: clone_into_zeroizing_32(&response.master_pk)?,
            chain_code: clone_into_zeroizing_32(&response.chain_code)?,
        };

        let session = SatsCardSweepSession::from_unseal(unseal_data, address, self.network);
        Ok(Arc::new(session))
    }

    async fn dump(&self, slot: u8) -> Result<SatsCardSlotDump> {
        let reader = self.reader.lock().await;

        let num_slots = reader.slots.1;
        if slot >= num_slots {
            return Err(SatsCardReaderError::SlotOutOfRange { slot, num_slots });
        }

        // Phase 1: dump-without-CVC only. Returns slot status (sealed/unsealed,
        // address for unsealed slots) but never key material across FFI. A
        // CVC-authenticated dump would return `privkey` for an already-unsealed
        // slot — that's a future addition that, like `unseal`, must hand back
        // an opaque `SatsCardSweepSession` rather than raw bytes.
        let response = reader.dump(slot as usize, None).await.map_err(TransportError::from)?;

        Ok(SatsCardSlotDump {
            slot: response.slot as u8,
            pubkey: response.pubkey,
            used: response.used,
            sealed: response.sealed,
            address: response.addr,
        })
    }
}

fn validate_cvc(cvc: &str) -> Result<()> {
    if cvc.len() != 6 {
        return Err(SatsCardReaderError::InvalidCvcLength(cvc.len() as u8));
    }

    if !cvc.chars().all(|c| c.is_ascii_digit()) {
        return Err(SatsCardReaderError::NonNumericCvc);
    }

    Ok(())
}

fn clone_into_zeroizing_32(bytes: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    if bytes.len() != 32 {
        return Err(SatsCardReaderError::Unknown(format!(
            "expected 32-byte field, got {}",
            bytes.len()
        )));
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(bytes);
    Ok(out)
}

// MARK: - FFI

/// Create a SatsCardReader instance for FFI callers.
///
/// `expected_url_suffix` should be the 8-character `r=…` value parsed
/// from the scanned `getsatscard.com/start#…` URL when available.
/// [`SatsCardReader::verified_address`] cross-checks against it.
///
/// UniFFI's Kotlin bindings do not support async primary constructors,
/// hence the free function.
#[uniffi::export]
pub async fn create_sats_card_reader(
    transport: Box<dyn TapcardTransportProtocol>,
    cmd: Option<SatsCardCmd>,
    expected_url_suffix: Option<String>,
) -> Result<Arc<SatsCardReader>, SatsCardReaderError> {
    let reader = SatsCardReader::new(transport, cmd, expected_url_suffix).await?;
    Ok(Arc::new(reader))
}

#[uniffi::export(name = "satsCardResponseStatusData")]
fn _ffi_sats_card_response_status(response: SatsCardResponse) -> Option<SatsCardStatus> {
    match response {
        SatsCardResponse::Status(status) => Some(status),
        _ => None,
    }
}

#[uniffi::export(name = "satsCardResponseUnsealSession")]
fn _ffi_sats_card_response_unseal(response: SatsCardResponse) -> Option<Arc<SatsCardSweepSession>> {
    match response {
        SatsCardResponse::Unseal(session) => Some(session),
        _ => None,
    }
}

#[uniffi::export(name = "satsCardResponseDumpData")]
fn _ffi_sats_card_response_dump(response: SatsCardResponse) -> Option<SatsCardSlotDump> {
    match response {
        SatsCardResponse::Dump(data) => Some(data),
        _ => None,
    }
}

// MARK: - Trait impls

impl std::hash::Hash for SatsCardReader {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Eq for SatsCardReader {}
impl PartialEq for SatsCardReader {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[uniffi::export]
impl SatsCardReaderError {
    #[uniffi::method(name = "isAuthError")]
    pub fn is_auth_error(&self) -> bool {
        matches!(self, Self::SatsCardError(TransportError::CkTap(CkTapError::BadAuth)))
    }

    #[uniffi::method(name = "isRateLimited")]
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::SatsCardError(TransportError::CkTap(CkTapError::RateLimited)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_cvc_accepts_six_digits() {
        assert!(validate_cvc("123456").is_ok());
    }

    #[test]
    fn validate_cvc_rejects_wrong_length() {
        assert!(matches!(validate_cvc("12345"), Err(SatsCardReaderError::InvalidCvcLength(5))));
        assert!(matches!(validate_cvc("1234567"), Err(SatsCardReaderError::InvalidCvcLength(7))));
    }

    #[test]
    fn validate_cvc_rejects_non_digit() {
        // and importantly, the malformed value is NOT echoed back through
        // Display — the variant has no payload.
        let err = validate_cvc("12345a").unwrap_err();
        assert!(matches!(err, SatsCardReaderError::NonNumericCvc));
        assert!(!err.to_string().contains("12345a"));
    }

    #[test]
    fn clone_into_zeroizing_32_rejects_wrong_length() {
        assert!(clone_into_zeroizing_32(&[0u8; 31]).is_err());
        assert!(clone_into_zeroizing_32(&[0u8; 32]).is_ok());
        assert!(clone_into_zeroizing_32(&[0u8; 33]).is_err());
    }

    #[test]
    fn unseal_data_is_not_uniffi_record() {
        // Compile-time guard: SatsCardUnsealData must remain crate-private
        // and must not implement uniffi::Record. If anyone re-derives
        // uniffi::Record on it, the `pub(crate)` access here will still
        // compile but the FFI surface will grow — review the security
        // model before changing this.
        fn _assert_crate_private(_: SatsCardUnsealData) {}
    }
}
