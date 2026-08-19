use std::{
    fmt,
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use bitcoin::{
    bip32::{Fingerprint, Xpub},
    hashes::HashEngine as _,
};
use nid::Nanoid;
use parking_lot::{Mutex as SyncMutex, RwLock};
use rust_cktap::{
    CkTapCard,
    shared::{Certificate as _, Wait as _},
    tap_signer::TapSignerShared as _,
};
use tokio::sync::Mutex;
use tracing::debug;

use crate::{
    database::Database,
    network::Network,
    psbt::Psbt,
    wallet::metadata::{
        TAP_SIGNER_ANNOUNCEMENT_HEIGHT, WalletBirthday, tap_signer_setup_birthday,
        valid_birth_height,
    },
};

use super::{CkTapError, TapcardTransport, TapcardTransportProtocol, TransportError};

const MAX_BACKUPS: u32 = 127;
const MAX_UNLUCKY_NUMBER_RETRIES: u8 = 3;

/// Errors returned when constructing a TAPSIGNER CVC
#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum TapSignerCvcError {
    /// The input contains a value other than an ASCII digit
    #[error("CVC must contain ASCII digits only")]
    InvalidCharacters,

    /// The input is outside the six to 32 digit protocol range
    #[error("CVC must contain between 6 and 32 ASCII digits")]
    InvalidLength(u32),
}

/// An opaque numeric CVC used to authenticate a TAPSIGNER
#[derive(Clone, PartialEq, Eq, uniffi::Object)]
pub struct TapSignerCvc(rust_cktap::Cvc);

impl Hash for TapSignerCvc {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_bytes().hash(state);
    }
}

impl fmt::Debug for TapSignerCvc {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted CVC>")
    }
}

impl TapSignerCvc {
    fn as_cktap(&self) -> &rust_cktap::Cvc {
        &self.0
    }
}

#[uniffi::export]
impl TapSignerCvc {
    /// Construct a CVC from six to 32 ASCII digits
    #[uniffi::constructor]
    pub fn try_new(value: String) -> Result<Self, TapSignerCvcError> {
        rust_cktap::Cvc::try_from(value).map(Self).map_err(Into::into)
    }
}

impl From<rust_cktap::CvcError> for TapSignerCvcError {
    fn from(error: rust_cktap::CvcError) -> Self {
        match error {
            rust_cktap::CvcError::TooShort { length }
            | rust_cktap::CvcError::TooLong { length } => Self::InvalidLength(length as u32),
            rust_cktap::CvcError::NonAsciiDigit { .. } => Self::InvalidCharacters,
        }
    }
}

/// Errors returned by a TAPSIGNER reader operation
#[derive(Debug, Clone, Hash, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum TapSignerReaderError {
    /// The card or transport reported a typed error
    #[error(transparent)]
    TapSignerError(#[from] TransportError),

    /// The PSBT could not be signed
    #[error("PsbtSignError: {0}")]
    PsbtSignError(String),

    /// The transaction could not be extracted from the PSBT
    #[error("ExtractTxError: {0}")]
    ExtractTxError(String),

    /// The connected card is not a TAPSIGNER
    #[error("UnknownCardType: {0}, expected TapSigner")]
    UnknownCardType(String),

    /// No command was supplied to the reader
    #[error("No command")]
    NoCommand,

    /// A setup command was supplied after setup completed
    #[error("Setup is already complete")]
    SetupAlreadyComplete,

    /// Another setup or mutating operation is already using this reader
    #[error("A TAPSIGNER operation is already in progress")]
    OperationInProgress,

    /// The continuation was already claimed by an earlier attempt
    #[error("Setup continuation was already used")]
    SetupContinuationAlreadyUsed,

    /// A continuation belongs to another verified card
    #[error("TAPSIGNER continuation belongs to another card")]
    ContinuationCardMismatch,

    /// The setup chain code is not exactly 32 bytes
    #[error("Invalid chain code length, must be 32, found {0}")]
    InvalidChainCodeLength(u32),

    /// The card has reached the protocol backup limit
    #[error("TAPSIGNER backup limit reached ({0})")]
    BackupLimitReached(u32),

    /// The card state cannot be determined safely
    #[error(
        "Unable to determine whether TAPSIGNER initialization completed; manual recovery is required"
    )]
    ManualRecoveryRequired,

    /// The backup command may have completed, but its bytes are not available
    #[error("TAPSIGNER backup data is unavailable; manual recovery is required")]
    BackupDataUnavailable,

    /// An error without a more specific public classification
    #[error("Unknown error: {0}")]
    Unknown(String),
}

type Error = TapSignerReaderError;
type Result<T, E = Error> = std::result::Result<T, E>;

/// A setup command with validated, typed authentication values
#[derive(Clone, Hash, PartialEq, Eq, uniffi::Object)]
pub struct SetupCmd {
    /// The factory CVC used during card initialization
    factory_cvc: Arc<TapSignerCvc>,
    /// The CVC stored on the card after setup
    new_cvc: Arc<TapSignerCvc>,
    /// The exact 32-byte chain code used during card initialization
    pub chain_code: [u8; 32],
}

impl fmt::Debug for SetupCmd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetupCmd")
            .field("factory_cvc", &"<redacted CVC>")
            .field("new_cvc", &"<redacted CVC>")
            .field("chain_code", &"<redacted>")
            .finish()
    }
}

#[uniffi::export]
impl SetupCmd {
    /// Build a setup command with an optional exact 32-byte chain code
    #[uniffi::constructor(default(chain_code = None))]
    pub fn try_new(
        factory_cvc: Arc<TapSignerCvc>,
        new_cvc: Arc<TapSignerCvc>,
        chain_code: Option<Vec<u8>>,
    ) -> Result<Self, Error> {
        let chain_code = match chain_code {
            Some(chain_code) => {
                let chain_code_len = chain_code.len() as u32;
                chain_code.try_into().map_err(|_| Error::InvalidChainCodeLength(chain_code_len))?
            }
            None => rust_cktap::rand_chaincode().to_bytes(),
        };

        Ok(Self { factory_cvc, new_cvc, chain_code })
    }
}

/// A command sent to a connected TAPSIGNER
#[derive(Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum TapSignerCmd {
    /// Initialize and set up the card
    Setup(Arc<SetupCmd>),
    /// Continue a previously uncertain setup stage
    ContinueSetup(Arc<TapSignerSetupContinuation>),
    /// Continue a previously uncertain standalone operation
    ContinueOperation(Arc<TapSignerOperationContinuation>),
    /// Create a card backup
    Backup { cvc: Arc<TapSignerCvc> },
    /// Derive the configured wallet key
    Derive { cvc: Arc<TapSignerCvc> },
    /// Change the current card CVC
    Change { current_cvc: Arc<TapSignerCvc>, new_cvc: Arc<TapSignerCvc> },
    /// Sign a PSBT with the card
    Sign { psbt: Arc<Psbt>, cvc: Arc<TapSignerCvc> },
}

impl fmt::Debug for TapSignerCmd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = match self {
            Self::Setup(_) => "setup",
            Self::ContinueSetup(_) => "continue_setup",
            Self::ContinueOperation(_) => "continue_operation",
            Self::Backup { .. } => "backup",
            Self::Derive { .. } => "derive",
            Self::Change { .. } => "change",
            Self::Sign { .. } => "sign",
        };

        formatter.debug_struct("TapSignerCmd").field("operation", &operation).finish()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum TapSignerOperation {
    Setup(Arc<SetupCmd>),
    ContinueSetup(Arc<TapSignerSetupContinuation>),
    ContinueOperation(Arc<TapSignerOperationContinuation>),
    Backup(Arc<TapSignerCvc>),
    Derive(Arc<TapSignerCvc>),
    Change { current_cvc: Arc<TapSignerCvc>, new_cvc: Arc<TapSignerCvc> },
    Sign { psbt: Arc<Psbt>, cvc: Arc<TapSignerCvc> },
}

impl TapSignerOperation {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Setup(_) => "setup",
            Self::ContinueSetup(_) => "continue_setup",
            Self::ContinueOperation(_) => "continue_operation",
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
            TapSignerCmd::ContinueSetup(continuation) => Ok(Self::ContinueSetup(continuation)),
            TapSignerCmd::ContinueOperation(continuation) => {
                Ok(Self::ContinueOperation(continuation))
            }
            TapSignerCmd::Backup { cvc } => Ok(Self::Backup(cvc)),
            TapSignerCmd::Derive { cvc } => Ok(Self::Derive(cvc)),
            TapSignerCmd::Change { current_cvc, new_cvc } => {
                Ok(Self::Change { current_cvc, new_cvc })
            }
            TapSignerCmd::Sign { psbt, cvc } => Ok(Self::Sign { psbt, cvc }),
        }
    }
}

/// A response from a TAPSIGNER operation
#[derive(Clone, PartialEq, Eq, uniffi::Enum, derive_more::From)]
pub enum TapSignerResponse {
    /// The setup operation returned a retry or completion response
    Setup(SetupCmdResponse),
    /// A standalone operation needs reconciliation before it can safely continue
    Retry(Arc<TapSignerOperationContinuation>),
    /// The card backup bytes
    Backup(Vec<u8>),
    /// The derived public key information
    Import(DeriveInfo),
    /// The CVC change completed
    Change,
    /// The signed PSBT
    Sign(Arc<Psbt>),
}

impl fmt::Debug for TapSignerResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setup(response) => formatter.debug_tuple("Setup").field(response).finish(),
            Self::Retry(continuation) => {
                formatter.debug_tuple("Retry").field(continuation).finish()
            }
            Self::Backup(_) => formatter.debug_tuple("Backup").field(&"<redacted>").finish(),
            Self::Import(response) => formatter.debug_tuple("Import").field(response).finish(),
            Self::Change => formatter.write_str("Change"),
            Self::Sign(_) => formatter.debug_tuple("Sign").field(&"<redacted>").finish(),
        }
    }
}

/// The successful result of TAPSIGNER setup
#[derive(Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct TapSignerSetupComplete {
    /// The encrypted card backup bytes
    pub backup: Vec<u8>,
    /// The derived key information
    pub derive_info: DeriveInfo,
    /// The wallet birthday selected for import
    pub birthday: WalletBirthday,
}

impl fmt::Debug for TapSignerSetupComplete {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TapSignerSetupComplete")
            .field("backup", &"<redacted>")
            .field("derive_info", &self.derive_info)
            .field("birthday", &self.birthday)
            .finish()
    }
}

/// Extended public key data returned by TAPSIGNER derivation
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct DeriveInfo {
    /// The serialized master public key
    pub master_pubkey: Vec<u8>,
    /// The serialized public key at the configured path
    pub pubkey: Vec<u8>,
    /// The current extended public key chain code
    pub chain_code: Vec<u8>,
    /// The non-hardened display form of the derivation path
    pub path: Vec<u32>,
    /// The network used for derivation
    pub network: Network,
    /// The card birth height when valid
    pub birth_height: Option<u64>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum SetupContinuationStage {
    ReconcileInit {
        cmd: Arc<SetupCmd>,
        error: TapSignerReaderError,
    },
    RetryBackup {
        cmd: Arc<SetupCmd>,
        pre_backup_count: Option<u32>,
        backup: Option<Vec<u8>>,
        error: TapSignerReaderError,
    },
    RetryDerive {
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        error: TapSignerReaderError,
    },
    RetryChange {
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        derive_info: DeriveInfo,
        error: TapSignerReaderError,
    },
    ReconcileChange {
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        derive_info: DeriveInfo,
        error: TapSignerReaderError,
    },
}

impl SetupContinuationStage {
    fn kind(&self) -> &'static str {
        match self {
            Self::ReconcileInit { .. } => "reconcile_init",
            Self::RetryBackup { .. } => "retry_backup",
            Self::RetryDerive { .. } => "retry_derive",
            Self::RetryChange { .. } => "retry_change",
            Self::ReconcileChange { .. } => "reconcile_change",
        }
    }

    fn error(&self) -> &TapSignerReaderError {
        match self {
            Self::ReconcileInit { error, .. }
            | Self::RetryBackup { error, .. }
            | Self::RetryDerive { error, .. }
            | Self::RetryChange { error, .. }
            | Self::ReconcileChange { error, .. } => error,
        }
    }

    fn with_error(&self, error: TapSignerReaderError) -> Self {
        match self {
            Self::ReconcileInit { cmd, .. } => Self::ReconcileInit { cmd: cmd.clone(), error },
            Self::RetryBackup { cmd, pre_backup_count, backup, .. } => Self::RetryBackup {
                cmd: cmd.clone(),
                pre_backup_count: *pre_backup_count,
                backup: backup.clone(),
                error,
            },
            Self::RetryDerive { cmd, backup, .. } => {
                Self::RetryDerive { cmd: cmd.clone(), backup: backup.clone(), error }
            }
            Self::RetryChange { cmd, backup, derive_info, .. } => Self::RetryChange {
                cmd: cmd.clone(),
                backup: backup.clone(),
                derive_info: derive_info.clone(),
                error,
            },
            Self::ReconcileChange { cmd, backup, derive_info, .. } => Self::ReconcileChange {
                cmd: cmd.clone(),
                backup: backup.clone(),
                derive_info: derive_info.clone(),
                error,
            },
        }
    }
}

/// Opaque continuation for an uncertain TAPSIGNER setup command
#[derive(uniffi::Object)]
pub struct TapSignerSetupContinuation {
    id: String,
    card_identity: String,
    stage: SetupContinuationStage,
    claimed: Arc<AtomicBool>,
}

impl TapSignerSetupContinuation {
    fn new(card_identity: String, stage: SetupContinuationStage) -> Arc<Self> {
        Arc::new(Self {
            id: Nanoid::<21>::new().to_string(),
            card_identity,
            stage,
            claimed: Arc::new(AtomicBool::new(false)),
        })
    }

    fn with_id(id: String, card_identity: String, stage: SetupContinuationStage) -> Arc<Self> {
        Arc::new(Self { id, card_identity, stage, claimed: Arc::new(AtomicBool::new(false)) })
    }

    fn retry_with_error(&self, error: TapSignerReaderError) -> Arc<Self> {
        Self::with_id(self.id.clone(), self.card_identity.clone(), self.stage.with_error(error))
    }

    fn with_stage(&self, stage: SetupContinuationStage) -> Arc<Self> {
        Self::with_id(self.id.clone(), self.card_identity.clone(), stage)
    }

    fn preserve_id(&self, response: SetupCmdResponse) -> SetupCmdResponse {
        match response {
            SetupCmdResponse::Retry(continuation) => {
                SetupCmdResponse::Retry(self.with_stage(continuation.stage.clone()))
            }
            SetupCmdResponse::Complete(complete) => SetupCmdResponse::Complete(complete),
        }
    }

    fn claim(&self) -> bool {
        self.claimed.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    fn stage(&self) -> &SetupContinuationStage {
        &self.stage
    }

    fn card_identity(&self) -> &str {
        &self.card_identity
    }
}

impl Clone for TapSignerSetupContinuation {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            card_identity: self.card_identity.clone(),
            stage: self.stage.clone(),
            claimed: self.claimed.clone(),
        }
    }
}

impl fmt::Debug for TapSignerSetupContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TapSignerSetupContinuation")
            .field("id", &self.id)
            .field("stage", &self.stage.kind())
            .finish()
    }
}

impl Hash for TapSignerSetupContinuation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl PartialEq for TapSignerSetupContinuation {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TapSignerSetupContinuation {}

#[uniffi::export]
impl TapSignerSetupContinuation {
    /// Return the stable id of this continuation
    pub fn id(&self) -> String {
        self.id.clone()
    }

    /// Return the typed error that caused this continuation
    pub fn error(&self) -> TapSignerReaderError {
        self.stage.error().clone()
    }

    /// Return a safe user-facing description of the continuation stage
    pub fn message(&self) -> String {
        match self.stage {
            SetupContinuationStage::ReconcileInit { .. } => {
                "TAPSIGNER initialization needs card status reconciliation".to_string()
            }
            SetupContinuationStage::RetryBackup { .. } => {
                "TAPSIGNER backup needs to be retried".to_string()
            }
            SetupContinuationStage::RetryDerive { .. } => {
                "TAPSIGNER derivation needs to be retried".to_string()
            }
            SetupContinuationStage::RetryChange { .. } => {
                "TAPSIGNER CVC change needs to be retried".to_string()
            }
            SetupContinuationStage::ReconcileChange { .. } => {
                "TAPSIGNER CVC change needs card status reconciliation".to_string()
            }
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
enum OperationContinuationStage {
    Backup {
        cvc: Arc<TapSignerCvc>,
        pre_backup_count: Option<u32>,
        backup: Option<Vec<u8>>,
        error: TapSignerReaderError,
    },
    Derive {
        cvc: Arc<TapSignerCvc>,
        error: TapSignerReaderError,
    },
    Change {
        current_cvc: Arc<TapSignerCvc>,
        new_cvc: Arc<TapSignerCvc>,
        error: TapSignerReaderError,
    },
}

impl OperationContinuationStage {
    fn error(&self) -> &TapSignerReaderError {
        match self {
            Self::Backup { error, .. }
            | Self::Derive { error, .. }
            | Self::Change { error, .. } => error,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Backup { .. } => "reconcile_backup",
            Self::Derive { .. } => "reconcile_derive",
            Self::Change { .. } => "reconcile_change",
        }
    }

    fn with_error(&self, error: TapSignerReaderError) -> Self {
        match self {
            Self::Backup { cvc, pre_backup_count, backup, .. } => Self::Backup {
                cvc: cvc.clone(),
                pre_backup_count: *pre_backup_count,
                backup: backup.clone(),
                error,
            },
            Self::Derive { cvc, .. } => Self::Derive { cvc: cvc.clone(), error },
            Self::Change { current_cvc, new_cvc, .. } => {
                Self::Change { current_cvc: current_cvc.clone(), new_cvc: new_cvc.clone(), error }
            }
        }
    }
}

/// Opaque continuation for an uncertain standalone TAPSIGNER operation
#[derive(uniffi::Object)]
pub struct TapSignerOperationContinuation {
    id: String,
    card_identity: String,
    stage: OperationContinuationStage,
    claimed: Arc<AtomicBool>,
}

impl TapSignerOperationContinuation {
    fn new(card_identity: String, stage: OperationContinuationStage) -> Arc<Self> {
        Arc::new(Self {
            id: Nanoid::<21>::new().to_string(),
            card_identity,
            stage,
            claimed: Arc::new(AtomicBool::new(false)),
        })
    }

    fn with_id(id: String, card_identity: String, stage: OperationContinuationStage) -> Arc<Self> {
        Arc::new(Self { id, card_identity, stage, claimed: Arc::new(AtomicBool::new(false)) })
    }

    fn retry_with_error(&self, error: TapSignerReaderError) -> Arc<Self> {
        Self::with_id(self.id.clone(), self.card_identity.clone(), self.stage.with_error(error))
    }

    fn with_stage(&self, stage: OperationContinuationStage) -> Arc<Self> {
        Self::with_id(self.id.clone(), self.card_identity.clone(), stage)
    }

    fn claim(&self) -> bool {
        self.claimed.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    fn card_identity(&self) -> &str {
        &self.card_identity
    }

    fn stage(&self) -> &OperationContinuationStage {
        &self.stage
    }
}

impl Clone for TapSignerOperationContinuation {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            card_identity: self.card_identity.clone(),
            stage: self.stage.clone(),
            claimed: self.claimed.clone(),
        }
    }
}

impl fmt::Debug for TapSignerOperationContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TapSignerOperationContinuation")
            .field("id", &self.id)
            .field("stage", &self.stage.kind())
            .finish()
    }
}

impl Hash for TapSignerOperationContinuation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl PartialEq for TapSignerOperationContinuation {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TapSignerOperationContinuation {}

#[uniffi::export]
impl TapSignerOperationContinuation {
    /// Return the stable id of this continuation
    pub fn id(&self) -> String {
        self.id.clone()
    }

    /// Return the typed error that caused this continuation
    pub fn error(&self) -> TapSignerReaderError {
        self.stage.error().clone()
    }

    /// Return whether this continuation can be passed to another retry attempt
    pub fn can_retry(&self) -> bool {
        !self.claimed.load(Ordering::Acquire)
    }

    /// Return whether this continuation stores the supplied CVC for a backup
    pub fn matches_backup(&self, cvc: Arc<TapSignerCvc>) -> bool {
        matches!(
            self.stage(),
            OperationContinuationStage::Backup { cvc: stored_cvc, .. }
                if stored_cvc == &cvc
        )
    }

    /// Return whether this continuation stores the supplied CVC for derivation
    pub fn matches_derive(&self, cvc: Arc<TapSignerCvc>) -> bool {
        matches!(
            self.stage(),
            OperationContinuationStage::Derive { cvc: stored_cvc, .. }
                if stored_cvc == &cvc
        )
    }

    /// Return whether this continuation stores both supplied CVCs for a change
    pub fn matches_change(
        &self,
        current_cvc: Arc<TapSignerCvc>,
        new_cvc: Arc<TapSignerCvc>,
    ) -> bool {
        matches!(
            self.stage(),
            OperationContinuationStage::Change {
                current_cvc: stored_current_cvc,
                new_cvc: stored_new_cvc,
                ..
            } if stored_current_cvc == &current_cvc && stored_new_cvc == &new_cvc
        )
    }

    /// Return a safe user-facing description of the continuation stage
    pub fn message(&self) -> String {
        match self.stage {
            OperationContinuationStage::Backup { .. } => {
                "TAPSIGNER backup needs card status reconciliation".to_string()
            }
            OperationContinuationStage::Derive { .. } => {
                "TAPSIGNER derivation needs card status reconciliation".to_string()
            }
            OperationContinuationStage::Change { .. } => {
                "TAPSIGNER CVC change needs card status reconciliation".to_string()
            }
        }
    }
}

/// The result of a setup attempt
#[derive(Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum SetupCmdResponse {
    /// The mobile layer must pass this opaque continuation to ContinueSetup
    Retry(Arc<TapSignerSetupContinuation>),
    /// Setup completed and returned the backup and derived key data
    Complete(TapSignerSetupComplete),
}

impl fmt::Debug for SetupCmdResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retry(continuation) => {
                formatter.debug_tuple("Retry").field(continuation).finish()
            }
            Self::Complete(complete) => formatter.debug_tuple("Complete").field(complete).finish(),
        }
    }
}

/// A verified TAPSIGNER reader
#[derive(uniffi::Object)]
pub struct TapSignerReader {
    id: String,
    card_identity: String,
    reader: Mutex<VerifiedTapSigner>,
    cmd: RwLock<Option<TapSignerOperation>>,
    transport: TapcardTransport,

    operation_guard: Mutex<()>,

    setup_done: AtomicBool,

    /// The latest retry or completion response
    last_response: SyncMutex<Option<Arc<TapSignerResponse>>>,

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
struct VerifiedTapSigner(rust_cktap::TapSigner);

impl VerifiedTapSigner {
    async fn connect(transport: TapcardTransport) -> Result<Self> {
        let card = rust_cktap::shared::to_cktap(Arc::new(transport.clone()))
            .await
            .map_err(TransportError::from)?;

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
        if matches!(root, rust_cktap::shared::FactoryRootKey::Dev(_)) {
            return Err(TransportError::IncorrectSignature(
                "TAPSIGNER uses a development factory certificate".to_string(),
            )
            .into());
        }

        Ok(Self(card))
    }
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
        let card_identity = card.card_ident();

        let me = Self {
            id: id.to_string(),
            card_identity,
            reader: Mutex::new(card),
            transport,
            cmd: RwLock::new(cmd),
            operation_guard: Mutex::new(()),
            setup_done: AtomicBool::new(false),
            last_response: SyncMutex::new(None),
            network,
        };

        me.wait_if_needed().await?;

        Ok(me)
    }
}

#[uniffi::export]
impl TapSignerReader {
    /// Execute the command supplied when this reader was created
    #[uniffi::method]
    pub async fn run(&self) -> Result<TapSignerResponse> {
        let _operation_guard = self
            .operation_guard
            .try_lock()
            .map_err(|_| TapSignerReaderError::OperationInProgress)?;
        let operation = self.cmd.write().take().ok_or(TapSignerReaderError::NoCommand)?;

        debug!(operation = operation.kind(), "running TapSigner operation");

        match operation {
            TapSignerOperation::Setup(cmd) => {
                Ok(TapSignerResponse::Setup(self.setup_inner(cmd).await?))
            }
            TapSignerOperation::ContinueSetup(continuation) => {
                Ok(TapSignerResponse::Setup(self.continue_setup(continuation).await?))
            }
            TapSignerOperation::ContinueOperation(continuation) => {
                self.continue_operation(continuation).await
            }
            TapSignerOperation::Backup(cvc) => self.run_backup(cvc).await,
            TapSignerOperation::Derive(cvc) => self.run_derive(cvc).await,
            TapSignerOperation::Change { current_cvc, new_cvc } => {
                self.run_change(current_cvc, new_cvc).await
            }
            TapSignerOperation::Sign { psbt, cvc } => {
                Ok(TapSignerResponse::Sign(self.sign_with_cvc(psbt, &cvc).await?.into()))
            }
        }
    }

    /// Start the setup process
    pub async fn setup(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse> {
        let _operation_guard = self
            .operation_guard
            .try_lock()
            .map_err(|_| TapSignerReaderError::OperationInProgress)?;

        self.setup_inner(cmd).await
    }

    /// Sign a PSBT with a typed TAPSIGNER CVC
    pub async fn sign(&self, psbt: Arc<Psbt>, cvc: Arc<TapSignerCvc>) -> Result<Psbt> {
        let _operation_guard = self
            .operation_guard
            .try_lock()
            .map_err(|_| TapSignerReaderError::OperationInProgress)?;

        self.sign_with_cvc(psbt, &cvc).await
    }

    /// Get the latest retry or completion response
    pub fn last_response(&self) -> Option<TapSignerResponse> {
        let response = self.last_response.lock().clone()?;
        Some(Arc::unwrap_or_clone(response))
    }
}

impl TapSignerReader {
    async fn setup_inner(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse> {
        if self.setup_complete() {
            return Err(TapSignerReaderError::SetupAlreadyComplete);
        }

        let initialized = self.reader.lock().await.path.is_some();
        if !initialized {
            match self.init_card(&cmd).await {
                Ok(()) => {}
                Err(error) if error.is_transport_uncertainty() => {
                    let continuation = TapSignerSetupContinuation::new(
                        self.card_identity.clone(),
                        SetupContinuationStage::ReconcileInit { cmd, error },
                    );
                    return Ok(self.record_retry(continuation));
                }
                Err(error) => return Err(error),
            }
        }

        self.backup_after_init(cmd).await
    }

    fn setup_complete(&self) -> bool {
        self.setup_done.load(Ordering::Acquire)
    }

    fn record_response(&self, response: TapSignerResponse) -> TapSignerResponse {
        *self.last_response.lock() = Some(Arc::new(response.clone()));
        response
    }

    fn record_setup_response(&self, response: SetupCmdResponse) -> SetupCmdResponse {
        let _ = self.record_response(TapSignerResponse::Setup(response.clone()));
        response
    }

    fn record_retry(&self, continuation: Arc<TapSignerSetupContinuation>) -> SetupCmdResponse {
        self.record_setup_response(SetupCmdResponse::Retry(continuation))
    }

    fn record_complete(&self, backup: Vec<u8>, derive_info: DeriveInfo) -> SetupCmdResponse {
        let birthday = tap_signer_setup_birthday(derive_info.network, derive_info.birth_height)
            .unwrap_or(WalletBirthday::BlockHeight(TAP_SIGNER_ANNOUNCEMENT_HEIGHT));
        let complete =
            SetupCmdResponse::Complete(TapSignerSetupComplete { backup, derive_info, birthday });

        self.setup_done.store(true, Ordering::Release);
        self.record_setup_response(complete)
    }

    fn record_operation_retry(
        &self,
        continuation: Arc<TapSignerOperationContinuation>,
    ) -> TapSignerResponse {
        self.record_response(TapSignerResponse::Retry(continuation))
    }

    fn continuation_matches(&self, card_identity: &str) -> bool {
        self.card_identity == card_identity
    }

    async fn run_backup(&self, cvc: Arc<TapSignerCvc>) -> Result<TapSignerResponse> {
        let pre_backup_count = self.reader.lock().await.num_backups;
        let backup = match self.backup(&cvc).await {
            Ok(backup) => backup,
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerOperationContinuation::new(
                    self.card_identity.clone(),
                    OperationContinuationStage::Backup {
                        cvc,
                        pre_backup_count,
                        backup: None,
                        error,
                    },
                );
                return Ok(self.record_operation_retry(continuation));
            }
            Err(error) => return Err(error),
        };

        match self.refresh_status().await {
            Ok(_) => Ok(self.record_response(TapSignerResponse::Backup(backup))),
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerOperationContinuation::new(
                    self.card_identity.clone(),
                    OperationContinuationStage::Backup {
                        cvc,
                        pre_backup_count,
                        backup: Some(backup),
                        error,
                    },
                );
                Ok(self.record_operation_retry(continuation))
            }
            Err(error) => Err(error),
        }
    }

    async fn run_derive(&self, cvc: Arc<TapSignerCvc>) -> Result<TapSignerResponse> {
        match self.derive(&cvc).await {
            Ok(derive_info) => Ok(self.record_response(TapSignerResponse::Import(derive_info))),
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerOperationContinuation::new(
                    self.card_identity.clone(),
                    OperationContinuationStage::Derive { cvc, error },
                );
                Ok(self.record_operation_retry(continuation))
            }
            Err(error) => Err(error),
        }
    }

    async fn run_change(
        &self,
        current_cvc: Arc<TapSignerCvc>,
        new_cvc: Arc<TapSignerCvc>,
    ) -> Result<TapSignerResponse> {
        match self.change(&new_cvc, &current_cvc).await {
            Ok(()) => {}
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerOperationContinuation::new(
                    self.card_identity.clone(),
                    OperationContinuationStage::Change { current_cvc, new_cvc, error },
                );
                return Ok(self.record_operation_retry(continuation));
            }
            Err(error) => return Err(error),
        }

        match self.refresh_status().await {
            Ok(_) => Ok(self.record_response(TapSignerResponse::Change)),
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerOperationContinuation::new(
                    self.card_identity.clone(),
                    OperationContinuationStage::Change { current_cvc, new_cvc, error },
                );
                Ok(self.record_operation_retry(continuation))
            }
            Err(error) => Err(error),
        }
    }

    async fn sign_with_cvc(&self, psbt: Arc<Psbt>, cvc: &Arc<TapSignerCvc>) -> Result<Psbt> {
        let psbt = Arc::unwrap_or_clone(psbt);
        let psbt: bitcoin::Psbt =
            self.reader.lock().await.sign_psbt(psbt.into(), cvc.as_cktap()).await.map_err(
                |error| match error {
                    rust_cktap::SignPsbtError::CkTap(error) => {
                        Error::from(TransportError::from(error))
                    }
                    error => Error::PsbtSignError(error.to_string()),
                },
            )?;

        Ok(psbt.into())
    }

    async fn wait_if_needed(&self) -> Result<()> {
        let mut reader = self.reader.lock().await;
        let mut auth_delay = reader.auth_delay;

        while let Some(delay) = auth_delay {
            let message = format!("Too many CVC attempts, waiting for {delay} seconds...");
            reader.wait(None).await.map_err(TransportError::from)?;
            self.transport.set_message(message);
            auth_delay = reader.auth_delay;
        }

        reader.auth_delay = None;
        Ok(())
    }

    async fn wait_with_cvc(&self, cvc: &TapSignerCvc) -> Result<()> {
        loop {
            self.refresh_status().await?;

            let auth_delay = self.reader.lock().await.auth_delay;
            if let Some(delay) = auth_delay {
                self.reader
                    .lock()
                    .await
                    .wait(None)
                    .await
                    .map_err(TransportError::from)
                    .map_err(Error::from)?;
                let message = format!("Too many CVC attempts, waiting for {delay} seconds...");
                self.transport.set_message(message);
                continue;
            }

            let auth_delay = self
                .reader
                .lock()
                .await
                .wait(Some(cvc.as_cktap().clone()))
                .await
                .map_err(TransportError::from)
                .map_err(Error::from)?;
            let Some(delay) = auth_delay else { return Ok(()) };

            let message = format!("Too many CVC attempts, waiting for {delay} seconds...");
            self.transport.set_message(message);
        }
    }

    async fn init_card(&self, cmd: &SetupCmd) -> Result<()> {
        for attempt in 0..=MAX_UNLUCKY_NUMBER_RETRIES {
            let result = self
                .reader
                .lock()
                .await
                .init(cmd.chain_code.into(), cmd.factory_cvc.as_cktap())
                .await
                .map_err(TransportError::from)
                .map_err(Error::from)
                .map_err(TapSignerReaderError::uncertain_after_mutation);

            match result {
                Err(error) if attempt < MAX_UNLUCKY_NUMBER_RETRIES && error.is_unlucky_number() => {
                    let (_, path) = self.refresh_status().await?;
                    if path.is_some() {
                        return Ok(());
                    }
                }
                Ok(()) => {
                    self.refresh_status().await?;
                    return Ok(());
                }
                result => return result,
            }
        }

        unreachable!("bounded init retry loop always returns")
    }

    async fn backup_after_init(&self, cmd: Arc<SetupCmd>) -> Result<SetupCmdResponse> {
        let pre_backup_count = self.reader.lock().await.num_backups;
        if let Some(count) = pre_backup_count.filter(|count| *count >= MAX_BACKUPS) {
            return Err(TapSignerReaderError::BackupLimitReached(count));
        }

        let backup = match self.backup(&cmd.factory_cvc).await {
            Ok(backup) => backup,
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerSetupContinuation::new(
                    self.card_identity.clone(),
                    SetupContinuationStage::RetryBackup {
                        cmd,
                        pre_backup_count,
                        backup: None,
                        error,
                    },
                );
                return Ok(self.record_retry(continuation));
            }
            Err(error) => return Err(error),
        };

        if let Err(error) = self.refresh_status().await {
            if error.is_transport_uncertainty() {
                let continuation = TapSignerSetupContinuation::new(
                    self.card_identity.clone(),
                    SetupContinuationStage::RetryBackup {
                        cmd,
                        pre_backup_count,
                        backup: Some(backup),
                        error,
                    },
                );
                return Ok(self.record_retry(continuation));
            }
            return Err(error);
        }

        self.derive_and_change(cmd, backup).await
    }

    async fn derive_and_change(
        &self,
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
    ) -> Result<SetupCmdResponse> {
        let derive_info = match self.derive(&cmd.factory_cvc).await {
            Ok(derive_info) => derive_info,
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerSetupContinuation::new(
                    self.card_identity.clone(),
                    SetupContinuationStage::RetryDerive { cmd, backup, error },
                );
                return Ok(self.record_retry(continuation));
            }
            Err(error) => return Err(error),
        };

        self.change_after_derive(cmd, backup, derive_info).await
    }

    async fn change_after_derive(
        &self,
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        derive_info: DeriveInfo,
    ) -> Result<SetupCmdResponse> {
        match self.change(&cmd.new_cvc, &cmd.factory_cvc).await {
            Ok(()) => {}
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerSetupContinuation::new(
                    self.card_identity.clone(),
                    SetupContinuationStage::ReconcileChange { cmd, backup, derive_info, error },
                );
                return Ok(self.record_retry(continuation));
            }
            Err(error) => return Err(error),
        }

        match self.refresh_status().await {
            Ok(_) => Ok(self.record_complete(backup, derive_info)),
            Err(error) if error.is_transport_uncertainty() => {
                let continuation = TapSignerSetupContinuation::new(
                    self.card_identity.clone(),
                    SetupContinuationStage::ReconcileChange { cmd, backup, derive_info, error },
                );
                Ok(self.record_retry(continuation))
            }
            Err(error) => Err(error),
        }
    }

    async fn continue_setup(
        &self,
        continuation: Arc<TapSignerSetupContinuation>,
    ) -> Result<SetupCmdResponse> {
        if self.setup_complete() {
            return Err(TapSignerReaderError::SetupAlreadyComplete);
        }
        if !self.continuation_matches(continuation.card_identity()) {
            return Err(TapSignerReaderError::ContinuationCardMismatch);
        }
        if !continuation.claim() {
            return Err(TapSignerReaderError::SetupContinuationAlreadyUsed);
        }

        match continuation.stage() {
            SetupContinuationStage::ReconcileInit { cmd, .. } => {
                self.reconcile_init(Arc::clone(&continuation), cmd.clone()).await
            }
            SetupContinuationStage::RetryBackup { cmd, pre_backup_count, backup, .. } => {
                self.retry_backup(
                    Arc::clone(&continuation),
                    cmd.clone(),
                    *pre_backup_count,
                    backup.clone(),
                )
                .await
            }
            SetupContinuationStage::RetryDerive { cmd, backup, .. } => {
                self.retry_derive(Arc::clone(&continuation), cmd.clone(), backup.clone()).await
            }
            SetupContinuationStage::RetryChange { cmd, backup, derive_info, .. } => {
                self.retry_change(
                    Arc::clone(&continuation),
                    cmd.clone(),
                    backup.clone(),
                    derive_info.clone(),
                )
                .await
            }
            SetupContinuationStage::ReconcileChange { cmd, backup, derive_info, .. } => {
                self.reconcile_change(
                    Arc::clone(&continuation),
                    cmd.clone(),
                    backup.clone(),
                    derive_info.clone(),
                )
                .await
            }
        }
    }

    async fn reconcile_init(
        &self,
        continuation: Arc<TapSignerSetupContinuation>,
        cmd: Arc<SetupCmd>,
    ) -> Result<SetupCmdResponse> {
        let (_, path) = match self.refresh_status().await {
            Ok(status) => status,
            Err(error) if error.is_card_error() => return Err(error),
            Err(_) => return Err(TapSignerReaderError::ManualRecoveryRequired),
        };

        if path.is_some() {
            let response = self.backup_after_init(cmd).await?;
            return Ok(self.record_setup_response(continuation.preserve_id(response)));
        }

        match self.init_card(&cmd).await {
            Ok(()) => {
                let response = self.backup_after_init(cmd).await?;
                Ok(self.record_setup_response(continuation.preserve_id(response)))
            }
            Err(error) if error.is_transport_uncertainty() => {
                Ok(self.record_retry(continuation.retry_with_error(error)))
            }
            Err(error) => Err(error),
        }
    }

    async fn retry_backup(
        &self,
        continuation: Arc<TapSignerSetupContinuation>,
        cmd: Arc<SetupCmd>,
        pre_backup_count: Option<u32>,
        backup: Option<Vec<u8>>,
    ) -> Result<SetupCmdResponse> {
        let (num_backups, path) = match self.refresh_status().await {
            Ok(status) => status,
            Err(error) if error.is_transport_uncertainty() => {
                return Ok(self.record_retry(continuation.retry_with_error(error)));
            }
            Err(error) => return Err(error),
        };

        if path.is_none() {
            return Err(TapSignerReaderError::ManualRecoveryRequired);
        }

        let Some(num_backups) = num_backups else {
            return Err(TapSignerReaderError::BackupDataUnavailable);
        };

        let backup_advanced = match pre_backup_count {
            Some(previous) => num_backups > previous,
            None => num_backups > 0,
        };
        if backup_advanced {
            if let Some(backup) = backup {
                let response = self.derive_and_change(cmd, backup).await?;
                return Ok(self.record_setup_response(continuation.preserve_id(response)));
            }
            return Err(TapSignerReaderError::BackupDataUnavailable);
        }
        if pre_backup_count.is_some_and(|previous| num_backups < previous) {
            return Err(TapSignerReaderError::BackupDataUnavailable);
        }

        if let Some(backup) = backup {
            let response = self.derive_and_change(cmd, backup).await?;
            return Ok(self.record_setup_response(continuation.preserve_id(response)));
        }
        if num_backups >= MAX_BACKUPS {
            return Err(TapSignerReaderError::BackupLimitReached(num_backups));
        }

        let backup = match self.backup(&cmd.factory_cvc).await {
            Ok(backup) => backup,
            Err(error) if error.is_transport_uncertainty() => {
                return Ok(self.record_retry(continuation.retry_with_error(error)));
            }
            Err(error) => return Err(error),
        };

        if let Err(error) = self.refresh_status().await {
            if error.is_transport_uncertainty() {
                return Ok(self.record_retry(continuation.with_stage(
                    SetupContinuationStage::RetryBackup {
                        cmd,
                        pre_backup_count: Some(num_backups),
                        backup: Some(backup),
                        error,
                    },
                )));
            }
            return Err(error);
        }

        let response = self.derive_and_change(cmd, backup).await?;
        Ok(self.record_setup_response(continuation.preserve_id(response)))
    }

    async fn retry_derive(
        &self,
        continuation: Arc<TapSignerSetupContinuation>,
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
    ) -> Result<SetupCmdResponse> {
        let derive_info = match self.derive(&cmd.factory_cvc).await {
            Ok(derive_info) => derive_info,
            Err(error) if error.is_transport_uncertainty() => {
                return Ok(self.record_retry(continuation.retry_with_error(error)));
            }
            Err(error) => return Err(error),
        };

        let response = self.change_after_derive(cmd, backup, derive_info).await?;
        Ok(self.record_setup_response(continuation.preserve_id(response)))
    }

    async fn retry_change(
        &self,
        continuation: Arc<TapSignerSetupContinuation>,
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        derive_info: DeriveInfo,
    ) -> Result<SetupCmdResponse> {
        match self.change(&cmd.new_cvc, &cmd.factory_cvc).await {
            Ok(()) => {}
            Err(error) if error.is_transport_uncertainty() => {
                let stage =
                    SetupContinuationStage::ReconcileChange { cmd, backup, derive_info, error };
                return Ok(self.record_retry(continuation.with_stage(stage)));
            }
            Err(error) => return Err(error),
        }

        match self.refresh_status().await {
            Ok(_) => Ok(self.record_complete(backup, derive_info)),
            Err(error) if error.is_transport_uncertainty() => {
                let stage =
                    SetupContinuationStage::ReconcileChange { cmd, backup, derive_info, error };
                Ok(self.record_retry(continuation.with_stage(stage)))
            }
            Err(error) => Err(error),
        }
    }

    async fn reconcile_change(
        &self,
        continuation: Arc<TapSignerSetupContinuation>,
        cmd: Arc<SetupCmd>,
        backup: Vec<u8>,
        derive_info: DeriveInfo,
    ) -> Result<SetupCmdResponse> {
        let wait_result = self.wait_with_cvc(&cmd.new_cvc).await;

        match wait_result {
            Ok(()) => Ok(self.record_complete(backup, derive_info)),
            Err(error) if error.is_auth_error() => {
                match self.wait_with_cvc(&cmd.factory_cvc).await {
                    Ok(()) => {
                        let stage =
                            SetupContinuationStage::RetryChange { cmd, backup, derive_info, error };
                        Ok(self.record_retry(continuation.with_stage(stage)))
                    }
                    Err(error) if error.is_transport_uncertainty() => {
                        Ok(self.record_retry(continuation.retry_with_error(error)))
                    }
                    Err(error) if error.is_auth_error() => {
                        Err(TapSignerReaderError::ManualRecoveryRequired)
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) if error.is_transport_uncertainty() => {
                Ok(self.record_retry(continuation.retry_with_error(error)))
            }
            Err(error) => Err(error),
        }
    }

    async fn continue_operation(
        &self,
        continuation: Arc<TapSignerOperationContinuation>,
    ) -> Result<TapSignerResponse> {
        if !self.continuation_matches(continuation.card_identity()) {
            return Err(TapSignerReaderError::ContinuationCardMismatch);
        }
        if !continuation.claim() {
            return Err(TapSignerReaderError::SetupContinuationAlreadyUsed);
        }

        match continuation.stage() {
            OperationContinuationStage::Backup { cvc, pre_backup_count, backup, .. } => {
                self.reconcile_operation_backup(
                    Arc::clone(&continuation),
                    cvc.clone(),
                    *pre_backup_count,
                    backup.clone(),
                )
                .await
            }
            OperationContinuationStage::Derive { cvc, .. } => match self.derive(cvc).await {
                Ok(derive_info) => Ok(self.record_response(TapSignerResponse::Import(derive_info))),
                Err(error) if error.is_transport_uncertainty() => {
                    Ok(self.record_operation_retry(continuation.retry_with_error(error)))
                }
                Err(error) => Err(error),
            },
            OperationContinuationStage::Change { current_cvc, new_cvc, .. } => {
                self.reconcile_operation_change(
                    Arc::clone(&continuation),
                    current_cvc.clone(),
                    new_cvc.clone(),
                )
                .await
            }
        }
    }

    async fn reconcile_operation_backup(
        &self,
        continuation: Arc<TapSignerOperationContinuation>,
        cvc: Arc<TapSignerCvc>,
        pre_backup_count: Option<u32>,
        backup: Option<Vec<u8>>,
    ) -> Result<TapSignerResponse> {
        let (num_backups, _) = match self.refresh_status().await {
            Ok(status) => status,
            Err(error) if error.is_transport_uncertainty() => {
                return Ok(self.record_operation_retry(continuation.retry_with_error(error)));
            }
            Err(error) => return Err(error),
        };
        let Some(num_backups) = num_backups else {
            return Err(TapSignerReaderError::BackupDataUnavailable);
        };
        let advanced = match pre_backup_count {
            Some(previous) => num_backups > previous,
            None => num_backups > 0,
        };
        if advanced && backup.is_none() {
            return Err(TapSignerReaderError::BackupDataUnavailable);
        }
        if pre_backup_count.is_some_and(|previous| num_backups < previous) {
            return Err(TapSignerReaderError::BackupDataUnavailable);
        }
        if let Some(backup) = backup {
            return Ok(self.record_response(TapSignerResponse::Backup(backup)));
        }
        if num_backups >= MAX_BACKUPS {
            return Err(TapSignerReaderError::BackupLimitReached(num_backups));
        }

        let backup = match self.backup(&cvc).await {
            Ok(backup) => backup,
            Err(error) if error.is_transport_uncertainty() => {
                return Ok(self.record_operation_retry(continuation.retry_with_error(error)));
            }
            Err(error) => return Err(error),
        };

        if let Err(error) = self.refresh_status().await {
            if error.is_transport_uncertainty() {
                let stage = OperationContinuationStage::Backup {
                    cvc,
                    pre_backup_count: Some(num_backups),
                    backup: Some(backup),
                    error,
                };
                return Ok(self.record_operation_retry(continuation.with_stage(stage)));
            }
            return Err(error);
        }

        Ok(self.record_response(TapSignerResponse::Backup(backup)))
    }

    async fn reconcile_operation_change(
        &self,
        continuation: Arc<TapSignerOperationContinuation>,
        current_cvc: Arc<TapSignerCvc>,
        new_cvc: Arc<TapSignerCvc>,
    ) -> Result<TapSignerResponse> {
        match self.wait_with_cvc(&new_cvc).await {
            Ok(()) => return Ok(self.record_response(TapSignerResponse::Change)),
            Err(error) if error.is_auth_error() => {}
            Err(error) if error.is_transport_uncertainty() => {
                return Ok(self.record_operation_retry(continuation.retry_with_error(error)));
            }
            Err(error) => return Err(error),
        }

        match self.wait_with_cvc(&current_cvc).await {
            Ok(()) => self.run_change(current_cvc, new_cvc).await,
            Err(error) if error.is_auth_error() => {
                Err(TapSignerReaderError::ManualRecoveryRequired)
            }
            Err(error) if error.is_transport_uncertainty() => {
                Ok(self.record_operation_retry(continuation.retry_with_error(error)))
            }
            Err(error) => Err(error),
        }
    }

    async fn refresh_status(&self) -> Result<(Option<u32>, Option<Vec<u32>>)> {
        let mut reader = self.reader.lock().await;
        let status = reader.status().await.map_err(TransportError::from).map_err(Error::from)?;

        let path = status.path.clone();
        let num_backups = status.num_backups;
        reader.path = path.clone();
        reader.num_backups = num_backups;
        reader.auth_delay = status.auth_delay;

        Ok((num_backups, path))
    }

    async fn backup(&self, cvc: &Arc<TapSignerCvc>) -> Result<Vec<u8>> {
        self.reader
            .lock()
            .await
            .backup(cvc.as_cktap())
            .await
            .map_err(TransportError::from)
            .map_err(Error::from)
            .map_err(TapSignerReaderError::uncertain_after_mutation)
    }

    async fn change(
        &self,
        new_cvc: &Arc<TapSignerCvc>,
        current_cvc: &Arc<TapSignerCvc>,
    ) -> Result<()> {
        debug!("starting CVC change");

        self.reader
            .lock()
            .await
            .change(new_cvc.as_cktap(), current_cvc.as_cktap())
            .await
            .map_err(TransportError::from)
            .map_err(Error::from)
            .map_err(TapSignerReaderError::uncertain_after_mutation)
    }

    async fn derive(&self, cvc: &Arc<TapSignerCvc>) -> Result<DeriveInfo> {
        debug!("starting derive");

        let path = self.target_path();
        let (_, current_path) = self.refresh_status().await?;
        if current_path.as_deref() != Some(path.as_slice()) {
            self.reader
                .lock()
                .await
                .derive(path.clone(), cvc.as_cktap())
                .await
                .map_err(TransportError::from)
                .map_err(Error::from)
                .map_err(TapSignerReaderError::uncertain_after_mutation)?;

            // Derive changes the card nonce and path. Read status again before xpub so the
            // authenticated xpub commands use the nonce returned by the card.
            self.refresh_status().await?;
        }

        let (birth_height, master_xpub, current_xpub) = {
            let mut reader = self.reader.lock().await;
            let birth_height = valid_birth_height(Some(u64::from(reader.birth)));

            let master_xpub = reader
                .xpub(true, cvc.as_cktap())
                .await
                .map_err(TransportError::from)
                .map_err(Error::from)
                .map_err(TapSignerReaderError::uncertain_after_mutation)?;
            let current_xpub = reader
                .xpub(false, cvc.as_cktap())
                .await
                .map_err(TransportError::from)
                .map_err(Error::from)
                .map_err(TapSignerReaderError::uncertain_after_mutation)?;

            (birth_height, master_xpub, current_xpub)
        };

        Ok(DeriveInfo::from_xpubs(master_xpub, current_xpub, path, self.network, birth_height))
    }

    fn target_path(&self) -> Vec<u32> {
        match self.network {
            Network::Bitcoin => vec![84, 0, 0],
            _ => vec![84, 1, 0],
        }
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

impl std::hash::Hash for TapSignerReader {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Eq for TapSignerReader {}
impl PartialEq for TapSignerReader {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl DeriveInfo {
    /// Build derivation data from the master and current extended public keys
    pub fn from_xpubs(
        master_xpub: Xpub,
        current_xpub: Xpub,
        path: Vec<u32>,
        network: Network,
        birth_height: Option<u64>,
    ) -> Self {
        Self {
            master_pubkey: master_xpub.public_key.serialize().to_vec(),
            pubkey: current_xpub.public_key.serialize().to_vec(),
            chain_code: current_xpub.chain_code.to_bytes().to_vec(),
            path,
            network,
            birth_height,
        }
    }

    /// Return the fingerprint of the serialized master public key
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
    /// Return the setup response when this is a setup result
    pub const fn setup_response(&self) -> Option<&SetupCmdResponse> {
        match self {
            Self::Setup(response) => Some(response),
            _ => None,
        }
    }

    /// Return a standalone operation continuation when this is a retry result
    pub fn retry_response(&self) -> Option<Arc<TapSignerOperationContinuation>> {
        match self {
            Self::Retry(continuation) => Some(Arc::clone(continuation)),
            _ => None,
        }
    }

    /// Return derivation data when this is an import result
    pub const fn derive_response(&self) -> Option<&DeriveInfo> {
        match self {
            Self::Import(response) => Some(response),
            _ => None,
        }
    }

    /// Return a marker when the CVC change completed
    pub const fn change_response(&self) -> Option<()> {
        match self {
            Self::Change => Some(()),
            _ => None,
        }
    }

    /// Return backup bytes when this is a backup result
    pub fn backup_response(&self) -> Option<&[u8]> {
        match self {
            Self::Backup(response) => Some(response),
            _ => None,
        }
    }

    /// Return the signed PSBT when this is a signing result
    pub fn sign_response(&self) -> Option<Arc<Psbt>> {
        match self {
            Self::Sign(txn) => Some(Arc::clone(txn)),
            _ => None,
        }
    }
}

impl TapSignerReaderError {
    fn uncertain_after_mutation(self) -> Self {
        match self {
            Self::TapSignerError(TransportError::CiborDe(message)) => {
                Self::TapSignerError(TransportError::Transport(format!(
                    "mutating response decode was uncertain: {message}"
                )))
            }
            Self::TapSignerError(TransportError::CiborValue(message)) => {
                Self::TapSignerError(TransportError::Transport(format!(
                    "mutating response decode was uncertain: {message}"
                )))
            }
            error => error,
        }
    }

    /// Return whether command completion is uncertain
    fn is_transport_uncertainty(&self) -> bool {
        matches!(
            self,
            Self::TapSignerError(
                TransportError::Transport(_)
                    | TransportError::CiborDe(_)
                    | TransportError::CiborValue(_)
            )
        )
    }

    fn is_card_error(&self) -> bool {
        matches!(self, Self::TapSignerError(TransportError::CkTap(_)))
    }

    pub const fn is_auth_error(&self) -> bool {
        matches!(self, Self::TapSignerError(TransportError::CkTap(CkTapError::BadAuth)))
    }

    pub const fn is_no_backup_error(&self) -> bool {
        matches!(self, Self::TapSignerError(TransportError::CkTap(CkTapError::BackupFirst)))
    }

    fn is_unlucky_number(&self) -> bool {
        matches!(self, Self::TapSignerError(TransportError::CkTap(CkTapError::UnluckyNumber)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, hash::DefaultHasher, sync::Arc};

    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use serde::Serialize;

    type MockResponseQueue = Arc<SyncMutex<VecDeque<Result<Vec<u8>, TransportError>>>>;

    #[derive(Debug, Serialize)]
    struct MockStatus {
        proto: u32,
        ver: String,
        birth: u32,
        slots: Option<(u8, u8)>,
        addr: Option<String>,
        tapsigner: Option<bool>,
        satschip: Option<bool>,
        path: Option<Vec<u32>>,
        num_backups: Option<u32>,
        #[serde(with = "serde_bytes")]
        pubkey: Vec<u8>,
        #[serde(with = "serde_bytes")]
        card_nonce: [u8; 16],
        testnet: Option<bool>,
        auth_delay: Option<u8>,
    }

    #[derive(Debug, Serialize)]
    struct MockErrorResponse {
        error: String,
        code: u16,
    }

    #[derive(Debug, Serialize)]
    struct MockWaitResponse {
        success: bool,
        auth_delay: u8,
    }

    #[derive(Debug, Serialize)]
    struct MockDeriveResponse {
        #[serde(with = "serde_bytes")]
        sig: [u8; 64],
        #[serde(with = "serde_bytes")]
        chain_code: [u8; 32],
        #[serde(with = "serde_bytes")]
        master_pubkey: [u8; 33],
        #[serde(with = "serde_bytes")]
        pubkey: Option<[u8; 33]>,
        #[serde(with = "serde_bytes")]
        card_nonce: [u8; 16],
    }

    #[derive(Debug, Serialize)]
    struct MockXpubResponse {
        #[serde(with = "serde_bytes")]
        xpub: Vec<u8>,
        #[serde(with = "serde_bytes")]
        card_nonce: [u8; 16],
    }

    #[derive(Debug)]
    struct MockStatusTransport {
        responses: MockResponseQueue,
        calls: Arc<SyncMutex<Vec<Vec<u8>>>>,
    }

    #[async_trait::async_trait]
    impl TapcardTransportProtocol for MockStatusTransport {
        fn set_message(&self, _message: String) {}

        fn append_message(&self, _message: String) {}

        async fn transmit_apdu(&self, command_apdu: Vec<u8>) -> Result<Vec<u8>, TransportError> {
            self.calls.lock().push(command_apdu);
            self.responses.lock().pop_front().unwrap_or_else(|| {
                Err(TransportError::Transport("mock response exhausted".to_string()))
            })
        }
    }

    fn status_response(num_backups: u32) -> Vec<u8> {
        status_response_with(Some(vec![84, 0, 0]), Some(num_backups))
    }

    fn uninitialized_status_response() -> Vec<u8> {
        status_response_with(None, None)
    }

    fn status_response_with(path: Option<Vec<u32>>, num_backups: Option<u32>) -> Vec<u8> {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[1; 32]).expect("valid test secret key");
        let pubkey = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let status = MockStatus {
            proto: 1,
            ver: "1.0.0".to_string(),
            birth: 700_000,
            slots: None,
            addr: None,
            tapsigner: Some(true),
            satschip: None,
            path,
            num_backups,
            pubkey: pubkey.serialize().to_vec(),
            card_nonce: [0; 16],
            testnet: None,
            auth_delay: None,
        };
        let mut response = Vec::new();
        ciborium::ser::into_writer(&status, &mut response).expect("status serializes");
        response
    }

    fn cbor_response<T: Serialize>(response: &T) -> Vec<u8> {
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(response, &mut encoded).expect("mock response serializes");
        encoded
    }

    fn bad_auth_response() -> Vec<u8> {
        cbor_response(&MockErrorResponse { error: "bad auth".to_string(), code: 401 })
    }

    fn unlucky_response() -> Vec<u8> {
        cbor_response(&MockErrorResponse { error: "unlucky".to_string(), code: 205 })
    }

    fn wait_success_response() -> Vec<u8> {
        cbor_response(&MockWaitResponse { success: true, auth_delay: 0 })
    }

    fn derive_response() -> Vec<u8> {
        let derive_info = ffi::derive_info();
        let master_pubkey = derive_info.master_pubkey.try_into().expect("valid master pubkey");
        let pubkey = derive_info.pubkey.try_into().expect("valid derived pubkey");
        cbor_response(&MockDeriveResponse {
            sig: [0; 64],
            chain_code: [42; 32],
            master_pubkey,
            pubkey: Some(pubkey),
            card_nonce: [1; 16],
        })
    }

    fn xpub_response(master: bool) -> Vec<u8> {
        use std::str::FromStr as _;

        let encoded = if master {
            "xpub661MyMwAqRbcFFr2SGY3dUn7g8P9VKNZdKWL2Z2pZMEkBWH2D1KTcwTn7keZQCaScCx7BUDjHFJJHnzBvDgUFgNjYsQTRvo7LWfYEtt78Pb"
        } else {
            "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM"
        };
        let xpub = Xpub::from_str(encoded).expect("valid test xpub");
        cbor_response(&MockXpubResponse { xpub: xpub.encode().to_vec(), card_nonce: [1; 16] })
    }

    fn reader_for_status(response: Vec<u8>) -> (TapSignerReader, Arc<SyncMutex<Vec<Vec<u8>>>>) {
        reader_for_responses(vec![Ok(response)], Some(vec![84, 0, 0]), Some(MAX_BACKUPS))
    }

    fn reader_for_responses(
        responses: Vec<Result<Vec<u8>, TransportError>>,
        path: Option<Vec<u32>>,
        num_backups: Option<u32>,
    ) -> (TapSignerReader, Arc<SyncMutex<Vec<Vec<u8>>>>) {
        reader_for_responses_with_identity(responses, path, num_backups, "TEST-CARD".to_string())
    }

    fn reader_for_responses_with_identity(
        responses: Vec<Result<Vec<u8>, TransportError>>,
        path: Option<Vec<u32>>,
        num_backups: Option<u32>,
        card_identity: String,
    ) -> (TapSignerReader, Arc<SyncMutex<Vec<Vec<u8>>>>) {
        let calls = Arc::new(SyncMutex::new(Vec::new()));
        let callback = MockStatusTransport {
            responses: Arc::new(SyncMutex::new(responses.into_iter().collect())),
            calls: calls.clone(),
        };
        let transport = TapcardTransport(Arc::new(Box::new(callback)));
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[1; 32]).expect("valid test secret key");
        let pubkey = bitcoin::PublicKey::new(bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp,
            &secret_key,
        ));
        let card = rust_cktap::TapSigner {
            transport: Arc::new(transport.clone()),
            secp,
            proto: 1,
            ver: "1.0.0".to_string(),
            birth: 700_000,
            path,
            num_backups,
            pubkey,
            card_nonce: [0; 16],
            auth_delay: None,
        };
        let reader = TapSignerReader {
            id: "test-reader".to_string(),
            card_identity,
            reader: Mutex::new(VerifiedTapSigner(card)),
            cmd: RwLock::new(None),
            transport,
            operation_guard: Mutex::new(()),
            setup_done: AtomicBool::new(false),
            last_response: SyncMutex::new(None),
            network: Network::Bitcoin,
        };

        (reader, calls)
    }

    fn command_field(command_apdu: &[u8], field: &str) -> Option<ciborium::Value> {
        let value: ciborium::Value =
            ciborium::de::from_reader(&command_apdu[5..]).expect("command is valid CBOR");
        let ciborium::Value::Map(entries) = value else { return None };

        entries.into_iter().find_map(|(key, value)| match key {
            ciborium::Value::Text(key) if key == field => Some(value),
            _ => None,
        })
    }

    fn command_names(calls: &SyncMutex<Vec<Vec<u8>>>) -> Vec<String> {
        calls
            .lock()
            .iter()
            .map(|command| match command_field(command, "cmd") {
                Some(ciborium::Value::Text(name)) => name,
                _ => "<unknown>".to_string(),
            })
            .collect()
    }

    #[test]
    fn cvc_errors_map_to_uniffi_domain() {
        assert_eq!(
            TapSignerCvcError::from(rust_cktap::CvcError::TooShort { length: 5 }),
            TapSignerCvcError::InvalidLength(5)
        );
        assert_eq!(
            TapSignerCvcError::from(rust_cktap::CvcError::NonAsciiDigit { index: 5 }),
            TapSignerCvcError::InvalidCharacters
        );
    }

    #[test]
    fn cvc_and_setup_values_are_redacted() {
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).expect("valid CVC"));
        assert!(!format!("{cvc:?}").contains("123456"));
        assert_eq!(format!("{cvc:?}"), "<redacted CVC>");

        let setup = SetupCmd::try_new(cvc.clone(), cvc, Some([0u8; 32].to_vec()))
            .expect("valid setup command");
        assert!(!format!("{setup:?}").contains("123456"));
    }

    #[test]
    fn setup_chain_code_is_optional_but_strictly_32_bytes_when_provided() {
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let generated = SetupCmd::try_new(cvc.clone(), cvc.clone(), None).unwrap();
        assert_eq!(generated.chain_code.len(), 32);

        assert!(matches!(
            SetupCmd::try_new(cvc.clone(), cvc.clone(), Some(vec![0; 31])),
            Err(TapSignerReaderError::InvalidChainCodeLength(31))
        ));
        assert!(matches!(
            SetupCmd::try_new(cvc.clone(), cvc, Some(vec![0; 33])),
            Err(TapSignerReaderError::InvalidChainCodeLength(33))
        ));
    }

    #[test]
    fn continuation_has_stable_id_and_redacted_debug() {
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).expect("valid CVC"));
        let setup = Arc::new(
            SetupCmd::try_new(cvc.clone(), cvc, Some([0u8; 32].to_vec()))
                .expect("valid setup command"),
        );
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::RetryBackup {
                cmd: setup,
                pre_backup_count: Some(0),
                backup: None,
                error: TapSignerReaderError::NoCommand,
            },
        );
        let cloned = continuation.clone();

        assert_eq!(continuation.id(), cloned.id());
        assert_eq!(continuation, cloned);
        assert!(matches!(continuation.stage(), SetupContinuationStage::RetryBackup { .. }));
        assert!(!format!("{continuation:?}").contains("123456"));
        assert_eq!(continuation.message(), "TAPSIGNER backup needs to be retried");

        let retried = continuation.retry_with_error(TapSignerReaderError::ManualRecoveryRequired);
        assert_eq!(retried.id(), continuation.id());
        assert_eq!(retried, continuation);
        assert_ne!(retried.error(), continuation.error());
        assert!(continuation.claim());
        assert!(!continuation.claim());
    }

    #[test]
    fn operation_continuation_retry_state_tracks_shared_claims() {
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let continuation = TapSignerOperationContinuation::new(
            "TEST-CARD".to_string(),
            OperationContinuationStage::Derive { cvc, error: TapSignerReaderError::NoCommand },
        );
        let cloned = continuation.clone();

        assert!(continuation.can_retry());
        assert!(cloned.can_retry());
        assert!(continuation.claim());
        assert!(!continuation.can_retry());
        assert!(!cloned.can_retry());
        assert!(!cloned.claim());

        let retried = continuation.retry_with_error(TapSignerReaderError::ManualRecoveryRequired);
        assert!(retried.can_retry());
    }

    #[test]
    fn operation_continuation_matches_complete_cvc_arguments() {
        let backup_cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let derive_cvc = Arc::new(TapSignerCvc::try_new("654321".to_string()).unwrap());
        let current_cvc = Arc::new(TapSignerCvc::try_new("112233".to_string()).unwrap());
        let new_cvc = Arc::new(TapSignerCvc::try_new("445566".to_string()).unwrap());
        let other_cvc = Arc::new(TapSignerCvc::try_new("778899".to_string()).unwrap());

        let backup = TapSignerOperationContinuation::new(
            "TEST-CARD".to_string(),
            OperationContinuationStage::Backup {
                cvc: backup_cvc.clone(),
                pre_backup_count: Some(1),
                backup: None,
                error: TapSignerReaderError::NoCommand,
            },
        );
        assert!(backup.matches_backup(backup_cvc.clone()));
        assert!(!backup.matches_backup(other_cvc.clone()));

        let derive = TapSignerOperationContinuation::new(
            "TEST-CARD".to_string(),
            OperationContinuationStage::Derive {
                cvc: derive_cvc.clone(),
                error: TapSignerReaderError::NoCommand,
            },
        );
        assert!(derive.matches_derive(derive_cvc.clone()));
        assert!(!derive.matches_derive(other_cvc.clone()));

        let change = TapSignerOperationContinuation::new(
            "TEST-CARD".to_string(),
            OperationContinuationStage::Change {
                current_cvc: current_cvc.clone(),
                new_cvc: new_cvc.clone(),
                error: TapSignerReaderError::NoCommand,
            },
        );
        assert!(change.matches_change(current_cvc.clone(), new_cvc.clone()));
        assert!(!change.matches_change(other_cvc.clone(), new_cvc.clone()));
        assert!(!change.matches_change(current_cvc, other_cvc));
    }

    #[test]
    fn reader_identity_hash_ignores_mutable_operation_state() {
        let (reader, _) = reader_for_responses(Vec::new(), Some(vec![84, 0, 0]), Some(0));
        let (other, _) = reader_for_responses(Vec::new(), None, None);

        assert_eq!(reader, other);

        let mut reader_hash = DefaultHasher::new();
        let mut other_hash = DefaultHasher::new();
        reader.hash(&mut reader_hash);
        other.hash(&mut other_hash);
        assert_eq!(reader_hash.finish(), other_hash.finish());
    }

    #[tokio::test]
    async fn continuation_card_mismatch_is_rejected_before_mutation() {
        let (reader, calls) = reader_for_responses_with_identity(
            Vec::new(),
            Some(vec![84, 0, 0]),
            Some(0),
            "OTHER-CARD".to_string(),
        );
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::RetryBackup {
                cmd: setup,
                pre_backup_count: Some(0),
                backup: None,
                error: TapSignerReaderError::ManualRecoveryRequired,
            },
        );

        let result = reader.continue_setup(continuation).await;

        assert!(matches!(result, Err(TapSignerReaderError::ContinuationCardMismatch)));
        assert!(calls.lock().is_empty());
    }

    #[tokio::test]
    async fn continuation_claim_is_shared_across_readers() {
        let (first, first_calls) = reader_for_status(status_response(MAX_BACKUPS));
        let (second, second_calls) =
            reader_for_responses(Vec::new(), Some(vec![84, 0, 0]), Some(0));
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::RetryBackup {
                cmd: setup,
                pre_backup_count: Some(MAX_BACKUPS),
                backup: None,
                error: TapSignerReaderError::ManualRecoveryRequired,
            },
        );

        let first_result = first.continue_setup(continuation.clone()).await;
        let second_result = second.continue_setup(continuation).await;

        assert!(matches!(first_result, Err(TapSignerReaderError::BackupLimitReached(MAX_BACKUPS))));
        assert!(matches!(second_result, Err(TapSignerReaderError::SetupContinuationAlreadyUsed)));
        assert_eq!(first_calls.lock().len(), 1);
        assert!(second_calls.lock().is_empty());
    }

    #[tokio::test]
    async fn operation_guard_rejects_overlapping_setup() {
        let (reader, calls) = reader_for_responses(Vec::new(), None, None);
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());
        let _guard = reader.operation_guard.try_lock().expect("test owns operation guard");

        let result = reader.setup(setup).await;

        assert!(matches!(result, Err(TapSignerReaderError::OperationInProgress)));
        assert!(calls.lock().is_empty());
    }

    #[tokio::test]
    async fn completed_setup_is_cached_and_rejects_new_setup() {
        let (reader, calls) = reader_for_responses(Vec::new(), Some(vec![84, 0, 0]), Some(1));
        let derive_info = ffi::derive_info();
        reader.setup_done.store(true, Ordering::Release);
        reader.record_response(TapSignerResponse::Setup(SetupCmdResponse::Complete(
            TapSignerSetupComplete {
                backup: vec![1, 2, 3],
                derive_info,
                birthday: WalletBirthday::BlockHeight(700_553),
            },
        )));
        reader.record_response(TapSignerResponse::Change);
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());

        let result = reader.setup(setup).await;

        assert!(matches!(result, Err(TapSignerReaderError::SetupAlreadyComplete)));
        assert!(calls.lock().is_empty());
    }

    #[tokio::test]
    async fn backup_count_advance_returns_data_unavailable_without_replay() {
        let (reader, calls) =
            reader_for_responses(vec![Ok(status_response(1))], Some(vec![84, 0, 0]), Some(1));
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::RetryBackup {
                cmd: setup,
                pre_backup_count: Some(0),
                backup: None,
                error: TapSignerReaderError::ManualRecoveryRequired,
            },
        );

        let result = reader.continue_setup(continuation).await;

        assert!(matches!(result, Err(TapSignerReaderError::BackupDataUnavailable)));
        assert_eq!(command_names(&calls), vec!["status"]);
    }

    #[tokio::test]
    async fn mutating_cbor_decode_is_an_uncertain_setup_result() {
        let (reader, calls) =
            reader_for_responses(vec![Ok(vec![0xff])], Some(vec![84, 0, 0]), Some(0));
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());

        let result = reader.setup(setup).await.unwrap();
        let SetupCmdResponse::Retry(continuation) = result else {
            panic!("backup decode uncertainty must return a continuation")
        };

        assert!(continuation.error().is_transport_uncertainty());
        assert_eq!(command_names(&calls), vec!["backup"]);
    }

    #[tokio::test]
    async fn derive_reconciliation_skips_applied_path_and_uses_fresh_status() {
        let responses =
            vec![Ok(status_response(0)), Ok(xpub_response(true)), Ok(xpub_response(false))];
        let (reader, calls) = reader_for_responses(responses, Some(vec![84, 0, 0]), Some(0));
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());

        reader.derive(&cvc).await.expect("xpub reads succeed");

        assert_eq!(command_names(&calls), vec!["status", "xpub", "xpub"]);
    }

    #[tokio::test]
    async fn derive_reconciliation_progresses_status_nonce_before_xpub() {
        let responses = vec![
            Ok(uninitialized_status_response()),
            Ok(derive_response()),
            Ok(status_response(0)),
            Ok(xpub_response(true)),
            Ok(xpub_response(false)),
        ];
        let (reader, calls) = reader_for_responses(responses, None, None);
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());

        reader.derive(&cvc).await.expect("derive and xpub reads succeed");

        assert_eq!(command_names(&calls), vec!["status", "derive", "status", "xpub", "xpub"]);
        let nonce = command_field(&calls.lock()[1], "nonce");
        assert!(matches!(nonce, Some(ciborium::Value::Bytes(bytes)) if bytes.len() == 16));
    }

    #[tokio::test]
    async fn standalone_change_decode_uncertainty_returns_operation_continuation() {
        let (reader, calls) =
            reader_for_responses(vec![Ok(vec![0xff])], Some(vec![84, 0, 0]), Some(1));
        let current_cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let new_cvc = Arc::new(TapSignerCvc::try_new("654321".to_string()).unwrap());
        *reader.cmd.write() = Some(TapSignerOperation::Change { current_cvc, new_cvc });

        let result = reader.run().await.unwrap();

        assert!(matches!(result, TapSignerResponse::Retry(_)));
        assert_eq!(command_names(&calls), vec!["change"]);
    }

    #[test]
    fn backup_bytes_are_redacted_from_public_debug_values() {
        let complete = TapSignerSetupComplete {
            backup: vec![0xde, 0xad, 0xbe, 0xef],
            derive_info: ffi::derive_info(),
            birthday: WalletBirthday::BlockHeight(700_553),
        };
        let response = TapSignerResponse::Backup(vec![0xde, 0xad, 0xbe, 0xef]);

        assert!(!format!("{complete:?}").contains("deadbeef"));
        assert!(!format!("{response:?}").contains("deadbeef"));
    }

    #[test]
    fn unknown_card_error_code_is_not_cbor_error() {
        let error = crate::tap_card::create_transport_error_from_code(499, "unknown".to_string());
        assert!(matches!(
            error,
            TransportError::UnknownCardErrorCode { code: 499, detail } if detail == "unknown"
        ));
    }

    #[test]
    fn card_and_transport_failures_remain_distinct() {
        let card =
            TransportError::from(rust_cktap::CkTapError::Card(rust_cktap::CardError::BadAuth));
        let transport =
            TransportError::from(rust_cktap::CkTapError::Transport("link lost".to_string()));

        assert!(matches!(card, TransportError::CkTap(CkTapError::BadAuth)));
        assert!(matches!(transport, TransportError::Transport(message) if message == "link lost"));
    }

    #[test]
    fn unknown_card_error_survives_transport_adapter() {
        let original = TransportError::UnknownCardErrorCode {
            code: 499,
            detail: "unknown card error".to_string(),
        };
        let round_trip = TransportError::from(rust_cktap::CkTapError::from(original.clone()));

        assert_eq!(round_trip, original);
    }

    #[test]
    fn apdu_status_word_remains_a_transport_failure_at_the_cktap_boundary() {
        let error = rust_cktap::CkTapError::from(TransportError::UnknownStatusWord {
            code: 0x6a80,
            detail: "incorrect parameters".to_string(),
        });

        assert!(matches!(
            error,
            rust_cktap::CkTapError::Transport(message)
                if message == "unknown APDU status word (0x6a80): incorrect parameters"
        ));
    }

    #[tokio::test]
    async fn backup_retry_at_protocol_limit_cannot_initialize() {
        let (reader, calls) = reader_for_status(status_response(MAX_BACKUPS));
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::RetryBackup {
                cmd: setup,
                pre_backup_count: Some(MAX_BACKUPS),
                backup: None,
                error: TapSignerReaderError::TapSignerError(TransportError::Transport(
                    "backup response was lost".to_string(),
                )),
            },
        );

        let result = reader.continue_setup(continuation.clone()).await;

        assert!(matches!(result, Err(TapSignerReaderError::BackupLimitReached(MAX_BACKUPS))));
        let replay = reader.continue_setup(continuation).await;

        assert!(matches!(replay, Err(TapSignerReaderError::SetupContinuationAlreadyUsed)));
        assert_eq!(calls.lock().len(), 1);
    }

    #[tokio::test]
    async fn backup_retry_does_not_initialize_an_uninitialized_card() {
        let (reader, calls) =
            reader_for_responses(vec![Ok(uninitialized_status_response())], None, None);
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::RetryBackup {
                cmd: setup,
                pre_backup_count: None,
                backup: None,
                error: TapSignerReaderError::TapSignerError(TransportError::Transport(
                    "backup response was lost".to_string(),
                )),
            },
        );

        let result = reader.continue_setup(continuation).await;

        assert!(matches!(result, Err(TapSignerReaderError::ManualRecoveryRequired)));
        assert_eq!(calls.lock().len(), 1);
    }

    #[tokio::test]
    async fn init_unlucky_number_retry_is_bounded() {
        let mut responses = Vec::new();
        for attempt in 0..=MAX_UNLUCKY_NUMBER_RETRIES {
            responses.push(Ok(unlucky_response()));
            if attempt < MAX_UNLUCKY_NUMBER_RETRIES {
                responses.push(Ok(uninitialized_status_response()));
            }
        }
        let (reader, calls) = reader_for_responses(responses, None, None);
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());

        let result = reader.setup(setup).await;

        assert!(matches!(
            result,
            Err(TapSignerReaderError::TapSignerError(TransportError::CkTap(
                CkTapError::UnluckyNumber
            )))
        ));
        assert_eq!(calls.lock().len(), usize::from(MAX_UNLUCKY_NUMBER_RETRIES) * 2 + 1);
    }

    #[tokio::test]
    async fn init_uncertainty_reconciles_before_backup_without_resending_init() {
        let responses = vec![
            Err(TransportError::Transport("init response was lost".to_string())),
            Ok(status_response(0)),
            Err(TransportError::Transport("backup response was lost".to_string())),
        ];
        let (reader, calls) = reader_for_responses(responses, None, None);
        let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let setup = Arc::new(SetupCmd::try_new(cvc.clone(), cvc, Some([0; 32].to_vec())).unwrap());

        let first = reader.setup(setup).await.unwrap();
        let continuation = match first {
            SetupCmdResponse::Retry(continuation) => continuation,
            SetupCmdResponse::Complete(_) => panic!("init uncertainty must return a retry"),
        };
        let second = reader.continue_setup(continuation).await.unwrap();

        assert!(
            matches!(second, SetupCmdResponse::Retry(continuation) if continuation.message() == "TAPSIGNER backup needs to be retried")
        );
        assert_eq!(calls.lock().len(), 3);
        assert_eq!(command_names(&calls), vec!["new", "status", "backup"]);
    }

    #[tokio::test]
    async fn derive_retry_preserves_backup_and_does_not_repeat_backup() {
        let responses = vec![
            Ok(status_response(1)),
            Ok(xpub_response(true)),
            Err(TransportError::Transport("current xpub response was lost".to_string())),
            Ok(status_response(1)),
            Ok(xpub_response(true)),
            Ok(xpub_response(false)),
            Err(TransportError::Transport("change response was lost".to_string())),
        ];
        let (reader, calls) = reader_for_responses(responses, Some(vec![84, 0, 0]), Some(1));
        let factory_cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let new_cvc = Arc::new(TapSignerCvc::try_new("654321".to_string()).unwrap());
        let setup =
            Arc::new(SetupCmd::try_new(factory_cvc, new_cvc, Some([0; 32].to_vec())).unwrap());
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::RetryDerive {
                cmd: setup,
                backup: vec![1, 2, 3],
                error: TapSignerReaderError::TapSignerError(TransportError::Transport(
                    "derive response was lost".to_string(),
                )),
            },
        );

        let first = reader.continue_setup(continuation).await.unwrap();
        let retry = match first {
            SetupCmdResponse::Retry(continuation) => continuation,
            SetupCmdResponse::Complete(_) => panic!("derive uncertainty must return a retry"),
        };
        assert_eq!(retry.message(), "TAPSIGNER derivation needs to be retried");
        let TapSignerResponse::Setup(SetupCmdResponse::Retry(stored)) =
            reader.last_response().expect("retry is stored")
        else {
            panic!("last response must store the retry")
        };
        assert_eq!(stored.id(), retry.id());

        let second = reader.continue_setup(retry).await.unwrap();
        let next = match second {
            SetupCmdResponse::Retry(continuation) => continuation,
            SetupCmdResponse::Complete(_) => panic!("change uncertainty must return a retry"),
        };
        assert_eq!(next.message(), "TAPSIGNER CVC change needs card status reconciliation");
        assert_eq!(calls.lock().len(), 7);
        assert_eq!(
            command_names(&calls),
            vec!["status", "xpub", "xpub", "status", "xpub", "xpub", "change"]
        );
    }

    #[tokio::test]
    async fn change_reconciliation_probes_old_cvc_before_retrying_change() {
        let responses = vec![
            Ok(status_response(0)),
            Ok(bad_auth_response()),
            Ok(status_response(0)),
            Ok(wait_success_response()),
        ];
        let (reader, calls) = reader_for_responses(responses, Some(vec![84, 0, 0]), Some(0));
        let factory_cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let new_cvc = Arc::new(TapSignerCvc::try_new("654321".to_string()).unwrap());
        let setup =
            Arc::new(SetupCmd::try_new(factory_cvc, new_cvc, Some([0; 32].to_vec())).unwrap());
        let derive_info = ffi::derive_info();
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::ReconcileChange {
                cmd: setup.clone(),
                backup: vec![1, 2, 3],
                derive_info: derive_info.clone(),
                error: TapSignerReaderError::TapSignerError(TransportError::Transport(
                    "change response was lost".to_string(),
                )),
            },
        );

        let retry = reader
            .reconcile_change(continuation.clone(), setup, vec![1, 2, 3], derive_info)
            .await
            .unwrap();
        let retry = match retry {
            SetupCmdResponse::Retry(continuation) => continuation,
            SetupCmdResponse::Complete(_) => panic!("new CVC probe must fail before retry"),
        };
        assert_eq!(retry.id(), continuation.id());
        assert_eq!(retry.message(), "TAPSIGNER CVC change needs to be retried");
        assert_eq!(calls.lock().len(), 4);
    }

    #[tokio::test]
    async fn change_reconciliation_requires_old_cvc_authentication() {
        let responses = vec![Ok(bad_auth_response()), Ok(bad_auth_response())];
        let (reader, _) = reader_for_responses(responses, Some(vec![84, 0, 0]), Some(0));
        let factory_cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let new_cvc = Arc::new(TapSignerCvc::try_new("654321".to_string()).unwrap());
        let setup =
            Arc::new(SetupCmd::try_new(factory_cvc, new_cvc, Some([0; 32].to_vec())).unwrap());
        let derive_info = ffi::derive_info();
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::ReconcileChange {
                cmd: setup.clone(),
                backup: vec![1, 2, 3],
                derive_info: derive_info.clone(),
                error: TapSignerReaderError::TapSignerError(TransportError::Transport(
                    "change response was lost".to_string(),
                )),
            },
        );

        let result = reader.reconcile_change(continuation, setup, vec![1, 2, 3], derive_info).await;

        assert!(matches!(result, Err(TapSignerReaderError::ManualRecoveryRequired)));
    }

    #[tokio::test]
    async fn change_reconciliation_retries_when_old_cvc_probe_is_uncertain() {
        let responses = vec![
            Ok(bad_auth_response()),
            Err(TransportError::Transport("old CVC probe was lost".to_string())),
        ];
        let (reader, _) = reader_for_responses(responses, Some(vec![84, 0, 0]), Some(0));
        let factory_cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).unwrap());
        let new_cvc = Arc::new(TapSignerCvc::try_new("654321".to_string()).unwrap());
        let setup =
            Arc::new(SetupCmd::try_new(factory_cvc, new_cvc, Some([0; 32].to_vec())).unwrap());
        let derive_info = ffi::derive_info();
        let continuation = TapSignerSetupContinuation::new(
            "TEST-CARD".to_string(),
            SetupContinuationStage::ReconcileChange {
                cmd: setup.clone(),
                backup: vec![1, 2, 3],
                derive_info: derive_info.clone(),
                error: TapSignerReaderError::TapSignerError(TransportError::Transport(
                    "change response was lost".to_string(),
                )),
            },
        );

        let result = reader
            .reconcile_change(continuation.clone(), setup, vec![1, 2, 3], derive_info)
            .await
            .unwrap();

        let retry = match result {
            SetupCmdResponse::Retry(continuation) => continuation,
            SetupCmdResponse::Complete(_) => panic!("uncertain old CVC probe must be retried"),
        };
        assert_eq!(retry.id(), continuation.id());
        assert_eq!(retry.message(), "TAPSIGNER CVC change needs card status reconciliation");
    }
}

mod ffi {
    use super::*;

    pub fn derive_info() -> DeriveInfo {
        use std::str::FromStr as _;

        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let original_xpub = Xpub::from_str(xpub).unwrap();
        let master_xpub = "xpub661MyMwAqRbcFFr2SGY3dUn7g8P9VKNZdKWL2Z2pZMEkBWH2D1KTcwTn7keZQCaScCx7BUDjHFJJHnzBvDgUFgNjYsQTRvo7LWfYEtt78Pb";
        let master_xpub = Xpub::from_str(master_xpub).unwrap();

        DeriveInfo::from_xpubs(
            master_xpub,
            original_xpub,
            vec![84, 1, 0],
            Network::Bitcoin,
            Some(700_553),
        )
    }
}

#[uniffi::export(name = "tapSignerResponseSetupResponse")]
fn _ffi_tap_signer_response_setup_response(
    response: TapSignerResponse,
) -> Option<SetupCmdResponse> {
    response.setup_response().cloned()
}

#[uniffi::export(name = "tapSignerResponseRetryResponse")]
fn _ffi_tap_signer_response_retry_response(
    response: TapSignerResponse,
) -> Option<Arc<TapSignerOperationContinuation>> {
    response.retry_response()
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

    let cvc = Arc::new(TapSignerCvc::try_new("123456".to_string()).expect("preview CVC is valid"));
    let setup_cmd = Arc::new(
        SetupCmd::try_new(cvc.clone(), cvc, Some([0u8; 32].to_vec()))
            .expect("preview setup command is valid"),
    );
    let continuation = TapSignerSetupContinuation::new(
        "PREVIEW-CARD".to_string(),
        SetupContinuationStage::RetryChange {
            cmd: setup_cmd,
            backup: vec![0u8; 32],
            derive_info: ffi::derive_info(),
            error: TapSignerReaderError::NoCommand,
        },
    );

    SetupCmdResponse::Retry(continuation)
}

#[uniffi::export]
impl TapSignerReaderError {
    /// Check whether this error means authentication failed
    #[uniffi::method(name = "isAuthError")]
    fn ffi_is_auth_error(&self) -> bool {
        self.is_auth_error()
    }

    /// Check whether this error means the card needs a backup first
    #[uniffi::method(name = "isNoBackupError")]
    fn ffi_is_no_backup_error(&self) -> bool {
        self.is_no_backup_error()
    }
}

// MARK: - FFI PREVIEW
#[uniffi::export(name = "tapSignerSetupCompleteNew")]
fn _ffi_tap_signer_setup_complete_new(preview: bool) -> TapSignerSetupComplete {
    assert!(preview);

    TapSignerSetupComplete {
        backup: vec![0u8; 32],
        derive_info: ffi::derive_info(),
        birthday: WalletBirthday::BlockHeight(700_553),
    }
}
