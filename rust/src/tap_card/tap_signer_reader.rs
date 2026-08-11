use std::{fmt, hash::Hasher, sync::Arc};

use bitcoin::{bip32::Fingerprint, hashes::HashEngine as _, secp256k1};
use nid::Nanoid;
use parking_lot::{Mutex as SyncMutex, RwLock};
use rust_cktap::{
    CkTapCard,
    apdu::DeriveResponse,
    commands::{Authentication, Certificate as _, CkTransport as _, Wait as _},
    factory_root_key::FactoryRootKey,
    tap_signer::TapSignerError,
};

use tokio::sync::Mutex;
use tracing::debug;
use zeroize::Zeroize;

use crate::{
    database::Database,
    network::Network,
    psbt::Psbt,
    wallet::metadata::{
        TAP_SIGNER_ANNOUNCEMENT_HEIGHT, WalletBirthday, tap_signer_setup_birthday,
        valid_birth_height,
    },
};
use cove_util::result_ext::ResultExt as _;

use super::{CkTapError, TapcardTransport, TapcardTransportProtocol, TransportError};

#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum TapSignerReaderError {
    #[error(transparent)]
    TapSignerError(#[from] TransportError),

    #[error("PsbtSignError: {0}")]
    PsbtSignError(String),

    #[error("ExtractTxError: {0}")]
    ExtractTxError(String),

    #[error("UnknownCardType: {0}, expected TapSigner")]
    UnknownCardType(String),

    #[error("No command")]
    NoCommand,

    #[error("PIN must be between 6 and 32 digits")]
    InvalidPinLength,

    #[error("PIN must contain only ASCII digits")]
    NonNumericPin,

    #[error("Setup is already complete")]
    SetupAlreadyComplete,

    #[error("Invalid chain code length, must be 32, found {0}")]
    InvalidChainCodeLength(u32),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

type Error = TapSignerReaderError;
type Result<T, E = Error> = std::result::Result<T, E>;

const MIN_PIN_LENGTH: usize = 6;
const MAX_PIN_LENGTH: usize = 32;

#[derive(Clone, Eq, Hash, PartialEq, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct TapSignerPin(String);

impl TapSignerPin {
    fn try_new(mut pin: String) -> Result<Self> {
        let pin_length = pin.len();
        if !(MIN_PIN_LENGTH..=MAX_PIN_LENGTH).contains(&pin_length) {
            pin.zeroize();
            return Err(TapSignerReaderError::InvalidPinLength);
        }

        if !pin.bytes().all(|byte| byte.is_ascii_digit()) {
            pin.zeroize();
            return Err(TapSignerReaderError::NonNumericPin);
        }

        Ok(Self(pin))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for TapSignerPin {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for TapSignerPin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted PIN>")
    }
}

// Main interface exposed to Swift
#[derive(uniffi::Object)]
pub struct TapSignerReader {
    id: String,
    reader: Mutex<VerifiedTapSigner>,
    cmd: RwLock<Option<TapSignerOperation>>,
    transport: TapcardTransport,

    /// Last response from the setup process, has started, if the last response is `Complete` then the setup process is complete
    last_response: SyncMutex<Option<Arc<SetupCmdResponse>>>,

    network: Network,
}

impl fmt::Debug for TapSignerReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = self.cmd.read().as_ref().map(TapSignerOperation::kind);

        formatter
            .debug_struct("TapSignerReader")
            .field("id", &self.id)
            .field("operation", &operation)
            .finish()
    }
}

#[derive(Debug, derive_more::Deref, derive_more::DerefMut)]
struct VerifiedTapSigner(rust_cktap::TapSigner<TapcardTransport>);

impl VerifiedTapSigner {
    async fn connect(transport: TapcardTransport) -> Result<Self> {
        let card = transport.clone().to_cktap().await.map_err(TransportError::from)?;
        let mut card = match card {
            CkTapCard::TapSigner(card) => Ok(card),
            CkTapCard::SatsCard(_) => {
                Err(TapSignerReaderError::UnknownCardType("SatsCard".to_string()))
            }
            CkTapCard::SatsChip(_) => {
                Err(TapSignerReaderError::UnknownCardType("SatsChip".to_string()))
            }
        }?;

        let root = card.check_certificate().await.map_err(TransportError::from)?;
        if matches!(root, FactoryRootKey::Dev(_)) {
            return Err(TransportError::IncorrectSignature(
                "TAPSIGNER uses a development factory certificate".to_string(),
            )
            .into());
        }

        Ok(Self(card))
    }
}

#[derive(Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum TapSignerCmd {
    Setup(Arc<SetupCmd>),
    Backup { pin: String },
    Derive { pin: String },
    Change { current_pin: String, new_pin: String },
    Sign { psbt: Arc<Psbt>, pin: String },
}

impl fmt::Debug for TapSignerCmd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = match self {
            Self::Setup(_) => "setup",
            Self::Backup { .. } => "backup",
            Self::Derive { .. } => "derive",
            Self::Change { .. } => "change",
            Self::Sign { .. } => "sign",
        };

        formatter.debug_struct("TapSignerCmd").field("operation", &operation).finish()
    }
}

#[derive(Clone, Hash, PartialEq, Eq, uniffi::Object)]
pub struct SetupCmd {
    factory_pin: TapSignerPin,
    new_pin: TapSignerPin,
    pub chain_code: [u8; 32],
}

impl fmt::Debug for SetupCmd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetupCmd")
            .field("factory_pin", &"<redacted PIN>")
            .field("new_pin", &"<redacted PIN>")
            .field("chain_code", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum TapSignerOperation {
    Setup(Arc<SetupCmd>),
    Backup(TapSignerPin),
    Derive(TapSignerPin),
    Change { current_pin: TapSignerPin, new_pin: TapSignerPin },
    Sign { psbt: Arc<Psbt>, pin: TapSignerPin },
}

impl TapSignerOperation {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Setup(_) => "setup",
            Self::Backup(_) => "backup",
            Self::Derive(_) => "derive",
            Self::Change { .. } => "change",
            Self::Sign { .. } => "sign",
        }
    }
}

impl TryFrom<TapSignerCmd> for TapSignerOperation {
    type Error = TapSignerReaderError;

    fn try_from(command: TapSignerCmd) -> Result<Self> {
        match command {
            TapSignerCmd::Setup(command) => Ok(Self::Setup(command)),
            TapSignerCmd::Backup { pin } => Ok(Self::Backup(TapSignerPin::try_new(pin)?)),
            TapSignerCmd::Derive { pin } => Ok(Self::Derive(TapSignerPin::try_new(pin)?)),
            TapSignerCmd::Change { current_pin, new_pin } => Ok(Self::Change {
                current_pin: TapSignerPin::try_new(current_pin)?,
                new_pin: TapSignerPin::try_new(new_pin)?,
            }),
            TapSignerCmd::Sign { psbt, pin } => {
                Ok(Self::Sign { psbt, pin: TapSignerPin::try_new(pin)? })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum, derive_more::From)]
pub enum TapSignerResponse {
    Setup(SetupCmdResponse),
    Backup(Vec<u8>),
    Import(DeriveInfo),
    Change,
    Sign(Arc<Psbt>),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum SetupCmdResponse {
    ContinueFromInit(ContinueFromInit),
    ContinueFromBackup(ContinueFromBackup),
    ContinueFromDerive(ContinueFromDerive),
    Complete(TapSignerSetupComplete),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromInit {
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromBackup {
    pub backup: Vec<u8>,
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct ContinueFromDerive {
    pub backup: Vec<u8>,
    pub derive_info: DeriveInfo,
    pub continue_cmd: Arc<SetupCmd>,
    pub error: TapSignerReaderError,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct TapSignerSetupComplete {
    pub backup: Vec<u8>,
    pub derive_info: DeriveInfo,
    pub birthday: WalletBirthday,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct DeriveInfo {
    pub master_pubkey: Vec<u8>,
    pub pubkey: Vec<u8>,
    pub chain_code: Vec<u8>,
    pub path: Vec<u32>,
    pub network: Network,
    pub birth_height: Option<u64>,
}

impl TapSignerReader {
    async fn new(
        transport: Box<dyn TapcardTransportProtocol>,
        cmd: Option<TapSignerCmd>,
    ) -> Result<Self> {
        let cmd = cmd.map(TapSignerOperation::try_from).transpose()?;
        let transport = TapcardTransport(Arc::new(transport));
        let card = VerifiedTapSigner::connect(transport.clone()).await?;

        debug!("TapSigner card authenticated");

        let id: Nanoid = Nanoid::new();
        let network = Database::global().global_config.selected_network();

        let me = Self {
            id: id.to_string(),
            reader: Mutex::new(card),
            transport,
            cmd: RwLock::new(cmd),
            last_response: SyncMutex::new(None),
            network,
        };

        // if the card has a required auth delay, wait for it
        me.wait_if_needed().await?;

        Ok(me)
    }
}

#[uniffi::export]
impl TapSignerReader {
    #[uniffi::method]
    pub async fn run(&self) -> Result<TapSignerResponse> {
        let operation = self.cmd.write().take().ok_or(TapSignerReaderError::NoCommand)?;

        debug!(operation = operation.kind(), "running TapSigner operation");

        match operation {
            TapSignerOperation::Setup(cmd) => {
                let response = self.setup(cmd).await?;
                Ok(TapSignerResponse::Setup(response))
            }

            TapSignerOperation::Backup(pin) => {
                let response = self.backup(&pin).await?;
                Ok(TapSignerResponse::Backup(response))
            }

            TapSignerOperation::Derive(pin) => {
                let response = self.derive(&pin).await?;
                Ok(TapSignerResponse::Import(response))
            }

            TapSignerOperation::Change { current_pin, new_pin } => {
                self.change(&new_pin, &current_pin).await?;
                Ok(TapSignerResponse::Change)
            }

            TapSignerOperation::Sign { psbt, pin } => {
                let txn = self.sign_with_pin(psbt, &pin).await?;
                Ok(TapSignerResponse::Sign(txn.into()))
            }
        }
    }

    /// Start the setup process
    pub async fn setup(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse, Error> {
        self.init_backup_change(cmd).await
    }

    /// User started the setup process, but errored out before completing the setup, we can continue from the last step
    pub async fn continue_setup(
        &self,
        response: SetupCmdResponse,
    ) -> Result<SetupCmdResponse, Error> {
        match response {
            SetupCmdResponse::ContinueFromInit(c) => self.init_backup_change(c.continue_cmd).await,

            SetupCmdResponse::ContinueFromBackup(c) => {
                let response = self.derive_and_change(c.continue_cmd, c.backup).await;
                Ok(response)
            }

            SetupCmdResponse::ContinueFromDerive(c) => {
                let response = self.setup_change_pin(c.continue_cmd, c.backup, c.derive_info).await;
                Ok(response)
            }

            // already complete, just return the backup
            SetupCmdResponse::Complete(c) => Ok(SetupCmdResponse::Complete(c)),
        }
    }

    pub async fn sign(&self, psbt: Arc<Psbt>, pin: &str) -> Result<Psbt, Error> {
        let pin = TapSignerPin::try_new(pin.to_owned())?;
        self.sign_with_pin(psbt, &pin).await
    }

    /// Get the last response from the reader
    pub fn last_response(&self) -> Option<TapSignerResponse> {
        let response = self.last_response.lock().clone()?;
        let response = Arc::unwrap_or_clone(response);
        let tap_signer_response = TapSignerResponse::Setup(response);
        Some(tap_signer_response)
    }
}

impl TapSignerReader {
    async fn sign_with_pin(&self, psbt: Arc<Psbt>, pin: &TapSignerPin) -> Result<Psbt, Error> {
        let psbt = Arc::unwrap_or_clone(psbt);

        let psbt: bitcoin::Psbt = self
            .reader
            .lock()
            .await
            .sign_psbt(psbt.into(), pin.as_str())
            .await
            .map_err_str(Error::PsbtSignError)?;

        Ok(psbt.into())
    }

    async fn wait_if_needed(&self) -> Result<(), Error> {
        let mut auth_delay = self.reader.lock().await.auth_delay;

        while let Some(delay) = auth_delay {
            let message = format!("Too many PIN attempts, waiting for {delay} seconds...");

            self.reader.lock().await.wait(None).await.map_err(TransportError::from)?;

            self.transport.set_message(message);
            auth_delay = self.reader.lock().await.auth_delay;
        }

        self.reader.lock().await.set_auth_delay(None);
        Ok(())
    }

    async fn init_backup_change(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse, Error> {
        let _init_response = self
            .reader
            .lock()
            .await
            .init(cmd.chain_code, cmd.factory_pin.as_str())
            .await
            .map_err(TransportError::from)?;

        Ok(self.backup_change_xpub(cmd).await)
    }

    async fn backup_change_xpub(&self, cmd: Arc<SetupCmd>) -> SetupCmdResponse {
        let backup_response = self.backup(&cmd.factory_pin).await;

        let backup = match backup_response {
            Ok(backup) => backup,
            Err(error) => {
                let response = SetupCmdResponse::ContinueFromInit(ContinueFromInit {
                    continue_cmd: cmd,
                    error,
                });

                *self.last_response.lock() = Some(response.clone().into());
                return response;
            }
        };

        self.derive_and_change(cmd.clone(), backup).await
    }

    async fn derive_and_change(&self, cmd: Arc<SetupCmd>, backup: Vec<u8>) -> SetupCmdResponse {
        let derive_info = match self.derive(&cmd.factory_pin).await {
            Ok(derive) => derive,
            Err(error) => {
                let response = SetupCmdResponse::ContinueFromBackup(ContinueFromBackup {
                    backup,
                    continue_cmd: cmd,
                    error,
                });

                *self.last_response.lock() = Some(response.clone().into());
                return response;
            }
        };

        self.setup_change_pin(cmd, backup, derive_info).await
    }

    async fn setup_change_pin(
        &self,
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        derive_info: DeriveInfo,
    ) -> SetupCmdResponse {
        debug!("starting pin change during setup");
        let change_response = self.change(&cmd.new_pin, &cmd.factory_pin).await;

        if let Err(error) = change_response {
            let response = SetupCmdResponse::ContinueFromDerive(ContinueFromDerive {
                backup,
                derive_info,
                continue_cmd: cmd,
                error,
            });

            *self.last_response.lock() = Some(response.clone().into());
            return response;
        }

        let birthday = tap_signer_setup_birthday(derive_info.network, derive_info.birth_height)
            .unwrap_or(WalletBirthday::BlockHeight(TAP_SIGNER_ANNOUNCEMENT_HEIGHT));

        let complete = TapSignerSetupComplete { backup, derive_info, birthday };

        *self.last_response.lock() = Some(SetupCmdResponse::Complete(complete.clone()).into());
        SetupCmdResponse::Complete(complete)
    }

    async fn backup(&self, pin: &TapSignerPin) -> Result<Vec<u8>, Error> {
        let backup_response =
            self.reader.lock().await.backup(pin.as_str()).await.map_err(TransportError::from)?;

        Ok(backup_response.data)
    }

    async fn change(
        &self,
        new_pin: &TapSignerPin,
        current_pin: &TapSignerPin,
    ) -> Result<(), Error> {
        debug!("starting pin change");

        self.reader
            .lock()
            .await
            .change(new_pin.as_str(), current_pin.as_str())
            .await
            .map_err(TransportError::from)?;

        Ok(())
    }

    async fn derive(&self, pin: &TapSignerPin) -> Result<DeriveInfo, Error> {
        debug!("starting derive");

        let path: [u32; 3] = match self.network {
            Network::Bitcoin => [84, 0, 0],
            _ => [84, 1, 0],
        };

        let (derive_response, birth_height) = {
            let mut reader = self.reader.lock().await;
            let birth_height = valid_birth_height(Some(
                reader.birth.try_into().expect("usize birth height fits in u64"),
            ));
            let derive_response = reader.derive(&path, pin.as_str()).await?;
            (derive_response, birth_height)
        };
        let derive_info =
            DeriveInfo::from_response(derive_response, path.to_vec(), self.network, birth_height);

        Ok(derive_info)
    }
}

/// Create a TapSignerReader instance for FFI callers
/// UniFFI's Kotlin bindings do not support async primary constructors
#[uniffi::export]
pub async fn create_tap_signer_reader(
    transport: Box<dyn TapcardTransportProtocol>,
    cmd: Option<TapSignerCmd>,
) -> Result<Arc<TapSignerReader>, TapSignerReaderError> {
    let reader = TapSignerReader::new(transport, cmd).await?;
    Ok(Arc::new(reader))
}

#[uniffi::export]
impl SetupCmd {
    #[uniffi::constructor(default(chain_code = None))]
    pub fn try_new(
        factory_pin: String,
        new_pin: String,
        chain_code: Option<Vec<u8>>,
    ) -> Result<Self, Error> {
        let factory_pin = TapSignerPin::try_new(factory_pin)?;
        let new_pin = TapSignerPin::try_new(new_pin)?;

        let chain_code = match chain_code {
            Some(chain_code) => {
                let chain_code_len = chain_code.len() as u32;
                chain_code.try_into().map_err(|_| Error::InvalidChainCodeLength(chain_code_len))?
            }
            None => rust_cktap::rand_chaincode(&mut secp256k1::rand::thread_rng()),
        };

        Ok(Self { factory_pin, new_pin, chain_code })
    }
}

impl std::hash::Hash for TapSignerReader {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.id, state);
        std::hash::Hash::hash(&self.cmd.read().as_ref(), state);
        std::hash::Hash::hash(&self.last_response.lock().as_ref(), state);
    }
}

impl Eq for TapSignerReader {}
impl PartialEq for TapSignerReader {
    fn eq(&self, other: &Self) -> bool {
        let response_lock = self.last_response.lock();
        let other_response_lock = other.last_response.lock();

        self.id == other.id
            && self.cmd.read().as_ref() == other.cmd.read().as_ref()
            && response_lock.as_ref() == other_response_lock.as_ref()
    }
}

impl DeriveInfo {
    pub fn from_response(
        derive_response: DeriveResponse,
        path: Vec<u32>,
        network: Network,
        birth_height: Option<u64>,
    ) -> Self {
        let master_pubkey = derive_response.master_pubkey;
        let chain_code = derive_response.chain_code;
        let pubkey = derive_response.pubkey.expect("has pubkey because path was given");

        Self {
            master_pubkey: master_pubkey.to_vec(),
            pubkey: pubkey.to_vec(),
            chain_code: chain_code.to_vec(),
            path,
            network,
            birth_height,
        }
    }

    pub fn master_fingerprint(&self) -> Fingerprint {
        use bitcoin::hashes::{Hash as _, ripemd160, sha256};

        let mut sha_engine = sha256::Hash::engine();
        sha_engine.input(self.master_pubkey.as_ref());
        let sha_result = sha256::Hash::from_engine(sha_engine);

        let mut ripemd_engine = ripemd160::Hash::engine();
        ripemd_engine.input(sha_result.as_ref());
        let hash160_result = ripemd160::Hash::from_engine(ripemd_engine);

        let mut fingerprint = [0u8; 4];
        fingerprint.copy_from_slice(&hash160_result[0..4]);

        Fingerprint::from(fingerprint)
    }
}

impl TapSignerResponse {
    pub const fn setup_response(&self) -> Option<&SetupCmdResponse> {
        match self {
            Self::Setup(response) => Some(response),
            _ => None,
        }
    }

    pub const fn derive_response(&self) -> Option<&DeriveInfo> {
        match self {
            Self::Import(response) => Some(response),
            _ => None,
        }
    }

    pub const fn change_response(&self) -> Option<()> {
        match self {
            Self::Change => Some(()),
            _ => None,
        }
    }

    pub fn backup_response(&self) -> Option<&[u8]> {
        match self {
            Self::Backup(response) => Some(response),
            _ => None,
        }
    }

    pub fn sign_response(&self) -> Option<Arc<Psbt>> {
        match self {
            Self::Sign(txn) => Some(Arc::clone(txn)),
            _ => None,
        }
    }
}

impl From<TapSignerError> for TapSignerReaderError {
    fn from(error: TapSignerError) -> Self {
        Self::TapSignerError(error.into())
    }
}

impl TapSignerReaderError {
    pub const fn is_auth_error(&self) -> bool {
        matches!(self, Self::TapSignerError(TransportError::CkTap(CkTapError::BadAuth)))
    }

    pub const fn is_no_backup_error(&self) -> bool {
        matches!(self, Self::TapSignerError(TransportError::CkTap(CkTapError::BackupFirst)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bitcoin::secp256k1::{
        Message, PublicKey, Secp256k1, SecretKey,
        hashes::{Hash as _, sha256},
    };
    use serde::{Deserialize, Serialize};

    use super::*;

    const CARD_NONCE: [u8; 16] = [7; 16];
    const NEXT_CARD_NONCE: [u8; 16] = [9; 16];

    #[derive(Debug)]
    struct CounterfeitTransport {
        calls: Arc<AtomicUsize>,
        card_secret: SecretKey,
    }

    #[derive(Serialize)]
    struct StatusResponse {
        proto: u8,
        ver: &'static str,
        birth: u32,
        tapsigner: bool,
        #[serde(with = "serde_bytes")]
        pubkey: Vec<u8>,
        #[serde(with = "serde_bytes")]
        card_nonce: [u8; 16],
    }

    #[derive(Serialize)]
    struct CertsResponse {
        cert_chain: Vec<Vec<u8>>,
    }

    #[derive(Deserialize)]
    struct CheckCommand {
        cmd: String,
        #[serde(with = "serde_bytes")]
        nonce: Vec<u8>,
    }

    #[derive(Serialize)]
    struct CheckResponse {
        #[serde(with = "serde_bytes")]
        auth_sig: Vec<u8>,
        #[serde(with = "serde_bytes")]
        card_nonce: [u8; 16],
    }

    impl CounterfeitTransport {
        fn new(calls: Arc<AtomicUsize>) -> Self {
            let card_secret = SecretKey::from_slice(&[3; 32]).expect("valid card secret");
            Self { calls, card_secret }
        }

        fn status_response(&self) -> Vec<u8> {
            let secp = Secp256k1::new();
            let pubkey = PublicKey::from_secret_key(&secp, &self.card_secret).serialize().to_vec();

            encode_response(&StatusResponse {
                proto: 1,
                ver: "1.0.0",
                birth: 700_000,
                tapsigner: true,
                pubkey,
                card_nonce: CARD_NONCE,
            })
        }

        fn check_response(&self, command_apdu: &[u8]) -> Vec<u8> {
            let command: CheckCommand =
                ciborium::de::from_reader(&command_apdu[5..]).expect("valid check command");
            assert_eq!(command.cmd, "check");

            let app_nonce: [u8; 16] = command.nonce.try_into().expect("16-byte app nonce");
            let message_bytes = [b"OPENDIME".as_slice(), &CARD_NONCE, &app_nonce].concat();
            let message = Message::from_digest(sha256::Hash::hash(&message_bytes).to_byte_array());
            let signature =
                Secp256k1::new().sign_ecdsa(&message, &self.card_secret).serialize_compact();

            encode_response(&CheckResponse {
                auth_sig: signature.to_vec(),
                card_nonce: NEXT_CARD_NONCE,
            })
        }
    }

    #[async_trait::async_trait]
    impl TapcardTransportProtocol for CounterfeitTransport {
        fn set_message(&self, _message: String) {}

        fn append_message(&self, _message: String) {}

        async fn transmit_apdu(
            &self,
            command_apdu: Vec<u8>,
        ) -> std::result::Result<Vec<u8>, TransportError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let response = match call {
                0 => self.status_response(),
                1 => encode_response(&CertsResponse { cert_chain: Vec::new() }),
                2 => self.check_response(&command_apdu),
                _ => panic!("authenticated command sent to counterfeit card"),
            };

            Ok(response)
        }
    }

    fn encode_response(response: &impl Serialize) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(response, &mut bytes).expect("response encodes");
        bytes
    }

    #[tokio::test]
    async fn rejects_counterfeit_before_authenticated_command() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CounterfeitTransport::new(Arc::clone(&calls));
        let command = TapSignerCmd::Backup { pin: "123456".to_string() };

        let error = TapSignerReader::new(Box::new(transport), Some(command))
            .await
            .expect_err("counterfeit card must be rejected");

        assert!(matches!(
            error,
            TapSignerReaderError::TapSignerError(TransportError::IncorrectSignature(message))
                if message.contains("counterfeit")
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn pin_errors_and_debug_output_do_not_include_pin_values() {
        for submitted_pin in ["123", "12345a", "１２３４５６"] {
            let error = TapSignerPin::try_new(submitted_pin.to_string()).unwrap_err();

            assert!(!error.to_string().contains(submitted_pin));
            assert!(!format!("{error:?}").contains(submitted_pin));
        }

        let pin = TapSignerPin::try_new("123456".to_string()).expect("valid PIN");
        assert_eq!(format!("{pin:?}"), "<redacted PIN>");
    }

    #[test]
    fn commands_and_setup_state_redact_pin_values() {
        let command = TapSignerCmd::Backup { pin: "123456".to_string() };
        assert!(!format!("{command:?}").contains("123456"));

        let setup =
            SetupCmd::try_new("123456".to_string(), "654321".to_string(), Some([0u8; 32].to_vec()))
                .expect("valid setup command");
        assert!(!format!("{setup:?}").contains("123456"));
        assert!(!format!("{setup:?}").contains("654321"));

        let response = SetupCmdResponse::ContinueFromInit(ContinueFromInit {
            continue_cmd: Arc::new(setup),
            error: TapSignerReaderError::NoCommand,
        });
        assert!(!format!("{response:?}").contains("123456"));
        assert!(!format!("{response:?}").contains("654321"));
    }
}

mod ffi {
    use super::*;

    pub fn derive_info() -> DeriveInfo {
        use std::str::FromStr as _;
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let original_xpub = bitcoin::bip32::Xpub::from_str(xpub).unwrap();

        let master_xpub = "xpub661MyMwAqRbcFFr2SGY3dUn7g8P9VKNZdKWL2Z2pZMEkBWH2D1KTcwTn7keZQCaScCx7BUDjHFJJHnzBvDgUFgNjYsQTRvo7LWfYEtt78Pb";
        let master_xpub = bitcoin::bip32::Xpub::from_str(master_xpub).unwrap();

        let master_xpub_bytes = master_xpub.public_key.serialize();
        let xpub_bytes = original_xpub.public_key.serialize();

        DeriveInfo {
            network: Network::Bitcoin,
            master_pubkey: master_xpub_bytes.to_vec(),
            pubkey: xpub_bytes.to_vec(),
            chain_code: original_xpub.chain_code.to_bytes().to_vec(),
            path: vec![84, 1, 0],
            birth_height: Some(700_553),
        }
    }
}

#[uniffi::export(name = "tapSignerResponseSetupResponse")]
fn _ffi_tap_signer_response_setup_response(
    response: TapSignerResponse,
) -> Option<SetupCmdResponse> {
    response.setup_response().cloned()
}

#[uniffi::export(name = "tapSignerResponseDeriveResponse")]
fn _ffi_tap_signer_response_derive_response(response: TapSignerResponse) -> Option<DeriveInfo> {
    response.derive_response().cloned()
}

#[uniffi::export(name = "tapSignerResponseChangeResponse")]
fn _ffi_tap_signer_response_change_response(response: TapSignerResponse) -> bool {
    response.change_response().is_some()
}

#[uniffi::export(name = "tapSignerResponseBackupResponse")]
fn _ffi_tap_signer_response_backup_response(response: TapSignerResponse) -> Option<Vec<u8>> {
    response.backup_response().map(Into::into)
}

#[uniffi::export(name = "tapSignerResponseSignResponse")]
fn _ffi_tap_signer_response_sign_response(response: TapSignerResponse) -> Option<Arc<Psbt>> {
    response.sign_response()
}

#[uniffi::export(name = "tapSignerSetupRetryContinueCmd")]
fn _ffi_tap_signer_setup_retry_continue_cmd(preview: bool) -> SetupCmdResponse {
    assert!(preview);

    let backup = vec![0u8; 32];
    let setup_cmd = SetupCmd::try_new(
        "123456".to_string(),
        "000000".to_string(),
        Some(cove_util::generate_random_chain_code().to_vec()),
    )
    .expect("preview PINs and chain code are valid");

    SetupCmdResponse::ContinueFromDerive(ContinueFromDerive {
        backup,
        derive_info: ffi::derive_info(),
        continue_cmd: Arc::new(setup_cmd),
        error: TapSignerReaderError::NoCommand,
    })
}

#[uniffi::export]
impl TapSignerReaderError {
    #[uniffi::method(name = "isAuthError")]
    fn ffi_is_auth_error(&self) -> bool {
        self.is_auth_error()
    }

    #[uniffi::method(name = "isNoBackupError")]
    fn ffi_is_no_backup_error(&self) -> bool {
        self.is_no_backup_error()
    }
}

// MARK: - FFI PREVIEW
#[uniffi::export(name = "tapSignerSetupCompleteNew")]
fn _ffi_tap_signer_setup_complete_new(preview: bool) -> TapSignerSetupComplete {
    assert!(preview);

    let backup = vec![0u8; 32];
    TapSignerSetupComplete {
        backup,
        derive_info: ffi::derive_info(),
        birthday: WalletBirthday::BlockHeight(700_553),
    }
}
