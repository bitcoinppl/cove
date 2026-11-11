use std::sync::Arc;

use minicbor::decode::{Decode, Decoder, Error as DecodeError};
use once_cell::sync::OnceCell;
use tracing::warn;

static REF: OnceCell<PasskeyAccess> = OnceCell::new();
const AUTH_DATA_FIELD: &str = "authData";

/// A failed passkey request
///
/// `Display` and `Debug` show only its [`PasskeyErrorDiagnostic`], so every log and error text
/// that carries one is redacted by its type
#[derive(Clone, Hash, Eq, PartialEq, uniffi::Error)]
#[uniffi::export(Display)]
pub enum PasskeyError {
    NotSupported {
        reason: PasskeyFailureReason,
    },

    PrfUnsupportedProvider,

    /// The user-started request did not complete, such as a dismissed sheet; never a missing
    /// credential
    NotCompleted,

    /// No foreground window or activity can present the request yet
    NotReady {
        operation: PasskeyOperation,
    },

    RequestFailed {
        operation: PasskeyOperation,
        reason: PasskeyFailureReason,
    },

    NoCredentialFound,
}

impl PasskeyError {
    /// The only form of this error that logs and error text may show
    pub fn diagnostic(&self) -> PasskeyErrorDiagnostic {
        let (operation, outcome, reason) = match self {
            Self::NotSupported { reason } => (None, "not_supported", Some(reason)),
            Self::PrfUnsupportedProvider => (None, "prf_unsupported", None),
            Self::NotCompleted => (None, "not_completed", None),
            Self::NotReady { operation } => (Some(*operation), "not_ready", None),
            Self::RequestFailed { operation, reason } => {
                (Some(*operation), reason.kind(), Some(reason))
            }
            Self::NoCredentialFound => (None, "no_credential", None),
        };

        let underlying = reason.and_then(PasskeyFailureReason::underlying);
        PasskeyErrorDiagnostic { operation, outcome, underlying }
    }
}

impl std::fmt::Display for PasskeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.diagnostic().fmt(f)
    }
}

impl std::fmt::Debug for PasskeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PasskeyError({})", self.diagnostic())
    }
}

impl std::error::Error for PasskeyError {}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum, thiserror::Error)]
#[uniffi::export(Display)]
pub enum PasskeyOperation {
    #[error("registration")]
    Registration,

    #[error("discover assertion")]
    DiscoverAssertion,

    #[error("authenticate assertion")]
    AuthenticateAssertion,
}

/// Why a passkey request failed
///
/// `Unknown` carries the platform's diagnostic to Rust for typing, but nothing formats it
/// directly: `Display` and `Debug` show the reason's kind and its [`PasskeyUnderlyingError`]
#[derive(Clone, Hash, Eq, PartialEq, uniffi::Enum)]
#[uniffi::export(Display)]
pub enum PasskeyFailureReason {
    PlatformAuthorizationFailed,
    PlatformAuthorizationFailedAfterPresentation,
    InvalidResponse,
    NotHandled,
    Interrupted,
    ProviderConfiguration,
    NoCreateOption,
    DeviceNotConfigured,
    UnexpectedCredentialType,
    MissingCredentialId,
    MalformedResponse,
    TimedOut,
    Unknown { diagnostic_message: String },
}

impl PasskeyFailureReason {
    /// A stable name for the reason, safe for logs
    pub fn kind(&self) -> &'static str {
        match self {
            Self::PlatformAuthorizationFailed => "authorization_failed",
            Self::PlatformAuthorizationFailedAfterPresentation => {
                "authorization_failed_after_presentation"
            }
            Self::InvalidResponse => "invalid_response",
            Self::NotHandled => "not_handled",
            Self::Interrupted => "interrupted",
            Self::ProviderConfiguration => "provider_configuration",
            Self::NoCreateOption => "no_create_option",
            Self::DeviceNotConfigured => "device_not_configured",
            Self::UnexpectedCredentialType => "unexpected_credential_type",
            Self::MissingCredentialId => "missing_credential_id",
            Self::MalformedResponse => "malformed_response",
            Self::TimedOut => "timed_out",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// The platform error behind an `Unknown` reason, as logs may show it
    pub fn underlying(&self) -> Option<PasskeyUnderlyingError> {
        match self {
            Self::Unknown { diagnostic_message } => {
                PasskeyUnderlyingError::parse(diagnostic_message)
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for PasskeyFailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind())?;
        match self.underlying() {
            Some(underlying) => write!(f, " {underlying}"),
            None => Ok(()),
        }
    }
}

impl std::fmt::Debug for PasskeyFailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PasskeyFailureReason({self})")
    }
}

impl std::error::Error for PasskeyFailureReason {}

/// What a log may say about a passkey error
///
/// Built only from the error's type and [`PasskeyUnderlyingError`], so platform text never
/// reaches it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasskeyErrorDiagnostic {
    pub operation: Option<PasskeyOperation>,
    pub outcome: &'static str,
    pub underlying: Option<PasskeyUnderlyingError>,
}

impl std::fmt::Display for PasskeyErrorDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "outcome={}", self.outcome)?;
        if let Some(operation) = self.operation {
            write!(f, " operation={operation:?}")?;
        }

        match self.underlying {
            Some(underlying) => write!(f, " {underlying}"),
            None => Ok(()),
        }
    }
}

/// Platform error domains whose names and codes are safe and useful in logs
///
/// The one allowlist for passkey logs: Rust's diagnostics and every native log line, through
/// [`passkey_request_log_line`], use it. Apple domains come from `NSError`; Android ones are the
/// `type` of an androidx `CredentialException`
const LOGGABLE_ERROR_DOMAINS: &[&str] = &[
    "com.apple.AuthenticationServices.AuthorizationError",
    "NSCocoaErrorDomain",
    "NSOSStatusErrorDomain",
    "android.credentials.GetCredentialException.TYPE_UNKNOWN",
    "android.credentials.GetCredentialException.TYPE_INTERRUPTED",
    "android.credentials.GetCredentialException.TYPE_USER_CANCELED",
    "androidx.credentials.TYPE_GET_CREDENTIAL_PROVIDER_CONFIGURATION_EXCEPTION",
    "androidx.credentials.TYPE_GET_CREDENTIAL_UNSUPPORTED_EXCEPTION",
];

/// A platform error behind a passkey failure, as logs may show it
///
/// An allowlisted domain keeps its name and code. Any other domain shows only that it was not
/// listed, so neither its name nor its code nor any free text reaches a log
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyUnderlyingError {
    Listed { domain: &'static str, code: Option<i64> },
    Unlisted,
}

impl PasskeyUnderlyingError {
    /// Checks a platform domain and code against the allowlist
    pub fn of(domain: &str, code: Option<i64>) -> Self {
        match LOGGABLE_ERROR_DOMAINS.iter().copied().find(|known| *known == domain) {
            Some(domain) => Self::Listed { domain, code },
            None => Self::Unlisted,
        }
    }

    /// Reads the `error_domain=<domain> error_code=<code>` form iOS sends in an `Unknown`
    /// reason; a diagnostic without a domain has no underlying error
    fn parse(diagnostic_message: &str) -> Option<Self> {
        let mut domain = None;
        let mut code = None;
        for field in diagnostic_message.split_whitespace() {
            if let Some(value) = field.strip_prefix("error_domain=") {
                domain = Some(value);
            } else if let Some(value) = field.strip_prefix("error_code=") {
                code = value.parse::<i64>().ok();
            }
        }

        Some(Self::of(domain?, code))
    }
}

impl std::fmt::Display for PasskeyUnderlyingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Listed { domain, code: Some(code) } => {
                write!(f, "error_domain={domain} error_code={code}")
            }
            Self::Listed { domain, code: None } => write!(f, "error_domain={domain}"),
            Self::Unlisted => f.write_str("error_domain=unlisted"),
        }
    }
}

/// The only form in which a native log may show a platform error behind a passkey request
///
/// iOS passes an `NSError`'s domain and code; Android passes a `CredentialException`'s `type`
/// and no code. Only an allowlisted domain comes back with its code
#[uniffi::export]
pub fn passkey_request_log_line(domain: String, code: Option<i64>) -> String {
    PasskeyUnderlyingError::of(&domain, code).to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PasskeyCredentialPresence {
    Present,
    Missing,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PasskeyRegistrationPlatform {
    Ios,
    Android,
}

#[derive(Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct PasskeyRegistrationResult {
    pub credential_id: Vec<u8>,
    pub provider_aaguid: String,
    pub registered_platform: PasskeyRegistrationPlatform,
}

impl std::fmt::Debug for PasskeyRegistrationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasskeyRegistrationResult")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("provider_aaguid", &"<redacted>")
            .field("registered_platform", &self.registered_platform)
            .finish()
    }
}

#[derive(Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct PasskeyRegistrationUser {
    pub id: Vec<u8>,
    pub name: String,
    pub display_name: String,
}

impl std::fmt::Debug for PasskeyRegistrationUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasskeyRegistrationUser")
            .field("id", &format_args!("<redacted len={}>", self.id.len()))
            .field("name", &self.name)
            .field("display_name", &self.display_name)
            .finish()
    }
}

/// Result from discovering a synced passkey during restore
#[derive(uniffi::Record)]
pub struct DiscoveredPasskeyResult {
    /// 32-byte PRF key
    pub prf_output: Vec<u8>,
    /// Discovered credential ID, persisted to local keychain
    pub credential_id: Vec<u8>,
}

impl std::fmt::Debug for DiscoveredPasskeyResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveredPasskeyResult")
            .field("prf_output", &format_args!("<redacted len={}>", self.prf_output.len()))
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .finish()
    }
}

#[uniffi::export(callback_interface)]
pub trait PasskeyProvider: Send + Sync + std::fmt::Debug + 'static {
    /// Create a new passkey credential
    fn create_passkey(
        &self,
        rp_id: String,
        challenge: Vec<u8>,
        user: PasskeyRegistrationUser,
    ) -> Result<PasskeyRegistrationResult, PasskeyError>;

    /// Authenticate with a known credential_id (enable flow, re-enable)
    fn authenticate_with_prf(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError>;

    /// Discoverable credential assertion — no credential_id needed
    ///
    /// Used during restore on a fresh device where local keychain is empty
    /// but the passkey is synced via iCloud Keychain.
    /// Returns both the 32-byte PRF output and the credential_id of the discovered passkey
    fn discover_and_authenticate_with_prf(
        &self,
        rp_id: String,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<DiscoveredPasskeyResult, PasskeyError>;

    fn is_prf_supported(&self) -> bool;

    /// Non-interactive check whether a passkey credential exists on the device
    ///
    /// Uses preferImmediatelyAvailableCredentials to silently detect absence
    /// without showing UI. Returns an indeterminate result when iOS fails or
    /// does not respond clearly enough to prove presence or absence
    fn check_passkey_presence(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
    ) -> PasskeyCredentialPresence;
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct PasskeyAccess(Arc<Box<dyn PasskeyProvider>>);

impl PasskeyAccess {
    pub fn global() -> &'static Self {
        REF.get().expect("passkey provider is not initialized")
    }
}

#[uniffi::export]
impl PasskeyAccess {
    #[uniffi::constructor]
    pub fn new(provider: Box<dyn PasskeyProvider>) -> Self {
        if let Some(me) = REF.get() {
            warn!("passkey provider is already initialized");
            return me.clone();
        }

        let me = Self(Arc::new(provider));
        REF.set(me).expect("failed to set passkey provider");

        Self::global().clone()
    }

    pub fn is_prf_supported(&self) -> bool {
        self.0.is_prf_supported()
    }
}

impl PasskeyAccess {
    pub fn create_passkey(
        &self,
        rp_id: String,
        challenge: Vec<u8>,
        user: PasskeyRegistrationUser,
    ) -> Result<PasskeyRegistrationResult, PasskeyError> {
        self.0.create_passkey(rp_id, challenge, user)
    }

    pub fn authenticate_with_prf(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError> {
        self.0.authenticate_with_prf(rp_id, credential_id, prf_salt, challenge)
    }

    pub fn discover_and_authenticate_with_prf(
        &self,
        rp_id: String,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<DiscoveredPasskeyResult, PasskeyError> {
        self.0.discover_and_authenticate_with_prf(rp_id, prf_salt, challenge)
    }

    pub fn check_passkey_presence(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
    ) -> PasskeyCredentialPresence {
        self.0.check_passkey_presence(rp_id, credential_id)
    }
}

#[uniffi::export]
pub fn passkey_aaguid_from_attestation_object(
    attestation_object: Vec<u8>,
) -> Result<String, PasskeyError> {
    extract_aaguid_from_attestation_object(&attestation_object).map_err(|reason| {
        PasskeyError::RequestFailed { operation: PasskeyOperation::Registration, reason }
    })
}

fn extract_aaguid_from_attestation_object(
    attestation_object: &[u8],
) -> Result<String, PasskeyFailureReason> {
    let auth_data = attestation_auth_data(attestation_object)?;
    extract_aaguid_from_auth_data(auth_data)
}

fn attestation_auth_data(attestation_object: &[u8]) -> Result<&[u8], PasskeyFailureReason> {
    let mut decoder = Decoder::new(attestation_object);
    let attestation_object = AttestationObject::decode(&mut decoder, &mut ())
        .map_err(|_| PasskeyFailureReason::MalformedResponse)?;

    Ok(attestation_object.auth_data)
}

struct AttestationObject<'a> {
    auth_data: &'a [u8],
}

impl<'b, C> Decode<'b, C> for AttestationObject<'b> {
    fn decode(decoder: &mut Decoder<'b>, _: &mut C) -> Result<Self, DecodeError> {
        let Some(map_len) = decoder.map()? else {
            return Err(DecodeError::message("indefinite attestation object"));
        };

        let mut auth_data = None;
        for _ in 0..map_len {
            match AttestationObjectField::from_key(decoder.str()?) {
                AttestationObjectField::AuthData => {
                    if auth_data.is_some() {
                        return Err(DecodeError::message("duplicate authData"));
                    }

                    auth_data = Some(decoder.bytes()?);
                }
                AttestationObjectField::Unknown => decoder.skip()?,
            }
        }

        let Some(auth_data) = auth_data else {
            return Err(DecodeError::message("missing authData"));
        };

        Ok(Self { auth_data })
    }
}

enum AttestationObjectField {
    AuthData,
    Unknown,
}

impl AttestationObjectField {
    fn from_key(key: &str) -> Self {
        match key {
            AUTH_DATA_FIELD => Self::AuthData,
            _ => Self::Unknown,
        }
    }
}

fn extract_aaguid_from_auth_data(auth_data: &[u8]) -> Result<String, PasskeyFailureReason> {
    const FLAGS_INDEX: usize = 32;
    const ATTESTED_CREDENTIAL_DATA_FLAG: u8 = 0x40;
    const AAGUID_START: usize = 37;
    const AAGUID_END: usize = AAGUID_START + 16;

    if auth_data.len() < AAGUID_END {
        return Err(PasskeyFailureReason::MalformedResponse);
    }

    if auth_data[FLAGS_INDEX] & ATTESTED_CREDENTIAL_DATA_FLAG == 0 {
        return Err(PasskeyFailureReason::InvalidResponse);
    }

    Ok(format_aaguid(&auth_data[AAGUID_START..AAGUID_END]))
}

fn format_aaguid(aaguid: &[u8]) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&aaguid[0..4]),
        hex::encode(&aaguid[4..6]),
        hex::encode(&aaguid[6..8]),
        hex::encode(&aaguid[8..10]),
        hex::encode(&aaguid[10..16])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkey_errors_show_only_their_redacted_diagnostic() {
        let unknown = |diagnostic_message: &str| PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: diagnostic_message.into() },
        };
        let listed = unknown("error_domain=NSCocoaErrorDomain error_code=4097");
        let unlisted =
            unknown("error_domain=com.example.passkey error_code=40442 user=alice@example.com");
        let no_domain = unknown("passkey creation failed for alice@example.com");

        assert_eq!(
            listed.to_string(),
            "outcome=unknown operation=AuthenticateAssertion error_domain=NSCocoaErrorDomain error_code=4097"
        );
        assert_eq!(
            unlisted.to_string(),
            "outcome=unknown operation=AuthenticateAssertion error_domain=unlisted"
        );
        assert_eq!(no_domain.to_string(), "outcome=unknown operation=AuthenticateAssertion");
        assert_eq!(
            PasskeyError::NotReady { operation: PasskeyOperation::Registration }.to_string(),
            "outcome=not_ready operation=Registration"
        );

        for formatted in [format!("{unlisted:?}"), format!("{no_domain:?}")] {
            for private in ["com.example.passkey", "40442", "alice@example.com"] {
                assert!(!formatted.contains(private), "{formatted}");
            }
        }
    }

    #[test]
    fn native_log_lines_use_the_same_allowlist() {
        assert_eq!(
            passkey_request_log_line(
                "com.apple.AuthenticationServices.AuthorizationError".into(),
                Some(1004)
            ),
            "error_domain=com.apple.AuthenticationServices.AuthorizationError error_code=1004"
        );
        assert_eq!(
            passkey_request_log_line(
                "android.credentials.GetCredentialException.TYPE_UNKNOWN".into(),
                None
            ),
            "error_domain=android.credentials.GetCredentialException.TYPE_UNKNOWN"
        );
        assert_eq!(
            passkey_request_log_line("com.example.passkey".into(), Some(40442)),
            "error_domain=unlisted"
        );
    }

    fn attestation_object(auth_data: &[u8]) -> Vec<u8> {
        let mut cbor = Vec::new();
        let mut encoder = minicbor::Encoder::new(&mut cbor);
        encoder.map(3).unwrap();
        encoder.str("fmt").unwrap();
        encoder.str("none").unwrap();
        encoder.str(AUTH_DATA_FIELD).unwrap();
        encoder.bytes(auth_data).unwrap();
        encoder.str("attStmt").unwrap();
        encoder.map(0).unwrap();

        cbor
    }

    #[test]
    fn extracts_auth_data_from_attestation_object() {
        let auth_data = [1, 2, 3, 4];
        let attestation_object = attestation_object(&auth_data);

        assert_eq!(attestation_auth_data(&attestation_object).unwrap(), auth_data);
    }

    #[test]
    fn discovered_passkey_debug_redacts_prf_output() {
        let result =
            DiscoveredPasskeyResult { prf_output: vec![10, 20, 30], credential_id: vec![1, 2, 3] };

        let debug = format!("{result:?}");

        assert!(debug.contains("prf_output: <redacted len=3>"));
        assert!(debug.contains("credential_id: <redacted len=3>"));
        assert!(!debug.contains("[10, 20, 30]"));
        assert!(!debug.contains("[1, 2, 3]"));
    }

    #[test]
    fn rejects_attestation_object_without_auth_data() {
        let mut cbor = Vec::new();
        let mut encoder = minicbor::Encoder::new(&mut cbor);
        encoder.map(1).unwrap();
        encoder.str("fmt").unwrap();
        encoder.str("none").unwrap();

        assert_eq!(attestation_auth_data(&cbor), Err(PasskeyFailureReason::MalformedResponse));
    }

    #[test]
    fn extracts_aaguid_from_auth_data() {
        let mut auth_data = vec![0u8; 37 + 16];
        auth_data[32] = 0x40;
        auth_data[37..53].copy_from_slice(&[
            0xea, 0x9b, 0x8d, 0x66, 0x4d, 0x01, 0x1d, 0x21, 0x3c, 0xe4, 0xb6, 0xb4, 0x8c, 0xb5,
            0x75, 0xd4,
        ]);

        assert_eq!(
            extract_aaguid_from_auth_data(&auth_data).unwrap(),
            "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4"
        );
    }

    #[test]
    fn rejects_auth_data_without_attested_credential_data() {
        let auth_data = vec![0u8; 37 + 16];

        assert_eq!(
            extract_aaguid_from_auth_data(&auth_data),
            Err(PasskeyFailureReason::InvalidResponse)
        );
    }
}
