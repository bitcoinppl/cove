//! Who may put a passkey sheet on screen
//!
//! A passkey request presents platform UI, so only a user's tap may start one, and only inside
//! the operation that tap started. The types carry that rule instead of each caller:
//! - a [`UserPermit`] is built only from a `PermitTicket`, which only `dispatch` mints, for the
//!   intents listed in `dispatch.rs`
//! - the operation record holds the permit's [`PermitGrant`]; the permit is live exactly while
//!   that grant exists, so ending, resetting or replacing the operation revokes every copy of the
//!   permit at once, and a nested operation or a continuation can hold a grant of its own
//! - [`PlatformAuthorizationRetrier`](super::PlatformAuthorizationRetrier) is the only code that
//!   calls the provider's interactive methods (a clippy `disallowed-methods` rule keeps it so). It
//!   reaches them only through a [`UserPermit`] it checks is still live before every request, so
//!   a request delayed past its operation never presents
//! - the platform asks [`passkey_request_may_present`] once its presenter is ready, right before
//!   it shows the sheet, so an operation that ends during the presenter wait presents nothing
//! - background work holds [`PasskeyRequestAuthority::Silent`]; every request it makes is
//!   refused with `NeedsPasskey` before reaching the provider
//! - [`PasskeyGate`] keeps one request on screen at a time; a second one answers `Busy` instead
//!   of queueing behind a sheet the user can't see

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cove_device::passkey::{PasskeyError, PasskeyOperation, PasskeyUnderlyingError};
use parking_lot::Mutex;
use tracing::debug;

use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::actors::PermitTicket;

/// The permit whose request is on screen, while one is
static PASSKEY_REQUEST_IN_FLIGHT: Mutex<Option<UserPermit>> = Mutex::new(None);

/// The tap a permit was granted for
///
/// Each one starts an operation whose described passkey sequence the tap authorizes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermitIntent {
    Enable,
    ConfirmSavedPasskey,
    RestoreFromCloudBackup,
    Verify,
    RepairPasskey,
    RecoverOtherBackups,
    /// The rebuilt manifest is verified, which authenticates the passkey
    RecreateManifest,
    /// Uses the permit only when the local master key is missing
    RestoreCloudWallet,
    /// Uses the permit only when the local master key is missing
    RestoreAll,
    /// A reinitialize registers the new backup's passkey
    Reinitialize,
    /// The delete is checked by a verification, which authenticates the passkey
    DeleteUndecryptableWalletBackups,
    /// The reinitialize after the user picks another Google Drive account
    SwitchDriveAccount,
}

/// The operation's side of a permit: the permit is live while the grant exists
///
/// The runtime keeps the grant inside the record of the operation the tap started, so the
/// permit's life is that record's life, with no separate bookkeeping to fall out of step
#[derive(Debug)]
pub(crate) struct PermitGrant {
    permit: UserPermit,
}

impl PermitGrant {
    /// Grants the permit for an admitted tap
    pub(crate) fn new(ticket: PermitTicket) -> Self {
        Self::for_intent(ticket.intent())
    }

    fn for_intent(intent: PermitIntent) -> Self {
        let live = Arc::new(AtomicBool::new(true));
        Self { permit: UserPermit { intent, live } }
    }

    /// A copy of the permit for the work the operation runs
    pub(crate) fn permit(&self) -> UserPermit {
        self.permit.clone()
    }
}

impl Drop for PermitGrant {
    fn drop(&mut self) {
        self.permit.live.store(false, Ordering::Release);
    }
}

/// A user's authority to present passkey requests inside one operation
///
/// Holding one proves only that a tap granted it; [`Self::check_live`] proves the operation it
/// was granted to still holds its [`PermitGrant`], which is what a request needs
#[derive(Debug, Clone)]
pub(crate) struct UserPermit {
    intent: PermitIntent,
    live: Arc<AtomicBool>,
}

impl UserPermit {
    pub(crate) fn intent(&self) -> PermitIntent {
        self.intent
    }

    /// Whether the operation this permit was granted to still holds its grant
    ///
    /// Read at the moment of the request, so a reset, a finish or a cancel that ended the
    /// operation revokes every copy of the permit at once
    pub(crate) fn check_live(&self) -> Result<(), PasskeyRequestRefusal> {
        if self.live.load(Ordering::Acquire) {
            return Ok(());
        }

        Err(PasskeyRequestRefusal::Revoked)
    }

    /// Clears one request to reach the provider: the permit is live and no sheet is on screen
    ///
    /// The provider call takes the guard this returns and holds it until the platform answers
    pub(crate) fn begin_request(&self) -> Result<PasskeyGateGuard, PasskeyRequestRefusal> {
        self.check_live()?;
        PasskeyGate::try_acquire(self)
    }
}

/// What may present a passkey request on behalf of the work that holds it
#[derive(Debug, Clone)]
pub(crate) enum PasskeyRequestAuthority {
    /// A user tap started this work, and its permit authorizes the requests the work makes
    Interactive(UserPermit),
    /// Background or follow-up work no tap authorized; it never presents a request
    Silent,
}

/// Why a passkey request was refused before it reached the platform
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasskeyRequestRefusal {
    /// Silent work asked for a passkey; the user has to start the work that needs it
    NeedsPasskey,
    /// The operation the permit belonged to ended
    Revoked,
    /// Another passkey request is still on screen
    Busy,
}

impl PasskeyRequestAuthority {
    /// The permit an interactive request needs; silent work is refused here
    pub(crate) fn permit(&self) -> Result<&UserPermit, PasskeyRequestRefusal> {
        match self {
            Self::Interactive(permit) => Ok(permit),
            Self::Silent => Err(PasskeyRequestRefusal::NeedsPasskey),
        }
    }
}

impl From<PasskeyRequestRefusal> for CloudBackupError {
    fn from(refusal: PasskeyRequestRefusal) -> Self {
        match refusal {
            PasskeyRequestRefusal::NeedsPasskey => Self::NeedsPasskey,
            PasskeyRequestRefusal::Revoked => Self::Cancelled,
            PasskeyRequestRefusal::Busy => Self::PasskeyBusy,
        }
    }
}

impl std::fmt::Display for PasskeyRequestRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsPasskey => write!(f, "background work can't present a passkey request"),
            Self::Revoked => write!(f, "the operation that held the passkey permit ended"),
            Self::Busy => write!(f, "another passkey request is on screen"),
        }
    }
}

/// Single-flight for platform passkey requests
///
/// The platform shows one sheet at a time, and a blocked UniFFI callback can't be interrupted
/// from Rust, so a request from an operation that already ended can still be on screen when a
/// new operation starts. The new request answers `Busy` instead of queueing behind it
#[derive(Debug)]
pub(crate) struct PasskeyGate;

/// Holds the gate until the platform request it guards returns
#[derive(Debug)]
pub(crate) struct PasskeyGateGuard(());

impl PasskeyGate {
    /// Takes the gate for `permit`'s request, or answers `Busy` while another request holds it
    ///
    /// The gate keeps the holder's permit, so a refusal can name the request on screen
    fn try_acquire(permit: &UserPermit) -> Result<PasskeyGateGuard, PasskeyRequestRefusal> {
        let mut holder = PASSKEY_REQUEST_IN_FLIGHT.lock();
        if let Some(open) = holder.as_ref() {
            debug!(
                "Passkey request for {:?} refused: {:?} is on screen",
                permit.intent, open.intent
            );
            return Err(PasskeyRequestRefusal::Busy);
        }

        *holder = Some(permit.clone());
        Ok(PasskeyGateGuard(()))
    }
}

impl Drop for PasskeyGateGuard {
    fn drop(&mut self) {
        *PASSKEY_REQUEST_IN_FLIGHT.lock() = None;
    }
}

/// Whether the passkey request the platform is about to show may still show
///
/// The platform asks after its presenter is ready and right before it starts the system sheet.
/// The operation that started the request can end while the platform waits for a presenter; the
/// request then answers false, and the platform reports it as not completed without showing
/// anything. Only interactive requests hold the gate, so a presence check doesn't ask
#[uniffi::export]
pub fn passkey_request_may_present() -> bool {
    let holder = PASSKEY_REQUEST_IN_FLIGHT.lock();
    holder.as_ref().is_some_and(|permit| permit.check_live().is_ok())
}

/// What a finished passkey request may say in logs
///
/// The kind of answer, the elapsed time and, for a failure, the error's own
/// [`PasskeyErrorDiagnostic`](cove_device::passkey::PasskeyErrorDiagnostic) outcome and
/// underlying error, so the one allowlist decides what reaches the log
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PasskeyRequestDiagnostic {
    pub(crate) operation: PasskeyOperation,
    pub(crate) outcome: &'static str,
    pub(crate) underlying: Option<PasskeyUnderlyingError>,
    pub(crate) elapsed: Duration,
}

impl PasskeyRequestDiagnostic {
    pub(crate) fn of<T>(
        operation: PasskeyOperation,
        result: &Result<Result<T, PasskeyError>, PasskeyRequestRefusal>,
        elapsed: Duration,
    ) -> Self {
        let (outcome, underlying) = match result {
            Ok(Ok(_)) => ("completed", None),
            Err(PasskeyRequestRefusal::NeedsPasskey) => ("needs_passkey", None),
            Err(PasskeyRequestRefusal::Revoked) => ("revoked", None),
            Err(PasskeyRequestRefusal::Busy) => ("busy", None),
            Ok(Err(error)) => {
                let diagnostic = error.diagnostic();
                (diagnostic.outcome, diagnostic.underlying)
            }
        };

        Self { operation, outcome, underlying, elapsed }
    }
}

impl std::fmt::Display for PasskeyRequestDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "operation={:?} outcome={}", self.operation, self.outcome)?;
        if let Some(underlying) = &self.underlying {
            write!(f, " {underlying}")?;
        }

        write!(f, " elapsed_ms={}", self.elapsed.as_millis())
    }
}

/// Permits for tests that drive a passkey consumer without the supervisor
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A permit whose operation stays live for the life of the process
    pub(crate) fn live_permit(intent: PermitIntent) -> PasskeyRequestAuthority {
        let (grant, authority) = revocable_permit(intent);
        std::mem::forget(grant);
        authority
    }

    /// A live permit, and the grant whose drop ends its operation
    pub(crate) fn revocable_permit(intent: PermitIntent) -> (PermitGrant, PasskeyRequestAuthority) {
        let grant = PermitGrant::for_intent(intent);
        let authority = PasskeyRequestAuthority::Interactive(grant.permit());
        (grant, authority)
    }

    /// Whether the request on screen now holds a live permit, or `None` with no request open,
    /// for the harness's invariant 6 samples
    pub(crate) fn passkey_gate_holder_live() -> Option<bool> {
        let holder = PASSKEY_REQUEST_IN_FLIGHT.lock();
        holder.as_ref().map(|permit| permit.check_live().is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::ops::test_support::async_test_lock;
    use cove_device::passkey::PasskeyFailureReason;

    #[test]
    fn a_request_may_present_only_while_its_operation_holds_the_grant() {
        // the gate is process-wide, so no other test may hold it meanwhile
        let _guard = async_test_lock().blocking_lock();
        assert!(!passkey_request_may_present(), "no request is open");

        let grant = PermitGrant::for_intent(PermitIntent::Verify);
        let guard = grant.permit().begin_request().expect("the gate is free");
        assert!(passkey_request_may_present());

        // the operation ends while the platform waits for a presenter
        drop(grant);
        assert!(!passkey_request_may_present());

        drop(guard);
        assert!(!passkey_request_may_present(), "the request closed");
    }

    fn unknown(message: &str) -> PasskeyError {
        PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: message.into() },
        }
    }

    #[test]
    fn diagnostics_keep_only_allowlisted_domains_and_their_codes() {
        let elapsed = Duration::from_millis(1_250);
        let allowed = PasskeyRequestDiagnostic::of(
            PasskeyOperation::AuthenticateAssertion,
            &Ok(Err::<(), _>(unknown(
                "error_domain=com.apple.AuthenticationServices.AuthorizationError error_code=1004",
            ))),
            elapsed,
        );
        let free_text = PasskeyRequestDiagnostic::of(
            PasskeyOperation::AuthenticateAssertion,
            &Ok(Err::<(), _>(unknown(
                "error_domain=com.example.Secret error_code=7 user=alice@example.com",
            ))),
            elapsed,
        );

        assert_eq!(
            allowed.to_string(),
            "operation=AuthenticateAssertion outcome=unknown error_domain=com.apple.AuthenticationServices.AuthorizationError error_code=1004 elapsed_ms=1250"
        );
        assert_eq!(
            free_text.to_string(),
            "operation=AuthenticateAssertion outcome=unknown error_domain=unlisted elapsed_ms=1250"
        );
    }
}
