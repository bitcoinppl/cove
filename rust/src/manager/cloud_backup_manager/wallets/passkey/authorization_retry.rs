use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backon::{BackoffBuilder as _, ExponentialBuilder, Retryable as _};
use cove_device::passkey::{
    DiscoveredPasskeyResult, PasskeyAccess, PasskeyError, PasskeyFailureReason, PasskeyOperation,
    PasskeyRegistrationResult, PasskeyRegistrationUser,
};
use cove_tokio::unblock;
use rand::RngExt as _;
use tokio::time::Instant;
use tracing::{info, warn};

#[cfg(test)]
use super::authority::PermitIntent;
use super::authority::{
    PasskeyGateGuard, PasskeyRequestAuthority, PasskeyRequestDiagnostic, PasskeyRequestRefusal,
    UserPermit,
};
use crate::manager::cloud_backup_manager::PASSKEY_RP_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformAuthorizationRetryPolicy {
    IosInteractive,
    LegacyDiscovery,
}

#[derive(Debug, Clone, Copy)]
struct PlatformAuthorizationRetryConfig {
    min_delay: Duration,
    max_delay: Duration,
    total_delay: Duration,
    jitter: bool,
}

impl PlatformAuthorizationRetryPolicy {
    fn for_current_platform() -> Self {
        if cfg!(target_os = "ios") { Self::IosInteractive } else { Self::LegacyDiscovery }
    }

    fn config(self) -> PlatformAuthorizationRetryConfig {
        match self {
            Self::IosInteractive => PlatformAuthorizationRetryConfig {
                min_delay: Duration::from_millis(500),
                max_delay: Duration::from_secs(4),
                total_delay: Duration::from_secs(15),
                jitter: true,
            },
            Self::LegacyDiscovery => PlatformAuthorizationRetryConfig {
                min_delay: Duration::from_millis(50),
                max_delay: Duration::from_secs(60),
                total_delay: Duration::from_secs(2),
                jitter: false,
            },
        }
    }

    fn retries(self, error: &PasskeyError) -> bool {
        let operation_is_in_scope = match self {
            Self::IosInteractive => true,
            Self::LegacyDiscovery => matches!(
                error,
                PasskeyError::RequestFailed { operation: PasskeyOperation::DiscoverAssertion, .. }
            ),
        };

        operation_is_in_scope && is_pre_presentation_platform_authorization_failure(error)
    }
}

pub(crate) fn is_pre_presentation_platform_authorization_failure(error: &PasskeyError) -> bool {
    matches!(
        error,
        PasskeyError::RequestFailed {
            reason: PasskeyFailureReason::PlatformAuthorizationFailed,
            ..
        }
    )
}

/// Presents passkey requests for one piece of work, under its authority
///
/// The only caller of the provider's interactive methods. Every request, first or retried, is
/// refused before it reaches the provider unless the work holds a permit whose operation is still
/// running and no other request is on screen. The retry budget covers transient platform
/// failures inside one permit and never outlives it
pub(crate) struct PlatformAuthorizationRetrier {
    authority: PasskeyRequestAuthority,
    policy: PlatformAuthorizationRetryPolicy,
    deadline: Instant,
    /// Set by the owning operation to stop retries and report the user cancelled
    cancellation: Arc<AtomicBool>,
    #[cfg(test)]
    jitter_seed: Option<u64>,
}

/// Why one attempt did not produce a credential
enum Attempt {
    Provider(PasskeyError),
    Refused(PasskeyRequestRefusal),
}

impl PlatformAuthorizationRetrier {
    /// A retrier for the work `authority` covers, which nothing else can cancel
    pub(crate) fn new(authority: PasskeyRequestAuthority) -> Self {
        Self::from_policy(
            authority,
            PlatformAuthorizationRetryPolicy::for_current_platform(),
            Arc::default(),
        )
    }

    /// Stops with `NotCompleted` once `cancellation` is set
    pub(crate) fn with_cancellation(mut self, cancellation: Arc<AtomicBool>) -> Self {
        self.cancellation = cancellation;
        self
    }

    fn from_policy(
        authority: PasskeyRequestAuthority,
        policy: PlatformAuthorizationRetryPolicy,
        cancellation: Arc<AtomicBool>,
    ) -> Self {
        Self {
            authority,
            policy,
            deadline: Instant::now() + policy.config().total_delay,
            cancellation,
            #[cfg(test)]
            jitter_seed: None,
        }
    }

    #[cfg(test)]
    fn for_test(policy: PlatformAuthorizationRetryPolicy, jitter_seed: u64) -> Self {
        let authority = super::authority::test_support::live_permit(PermitIntent::Verify);
        Self::for_test_with(authority, policy, jitter_seed)
    }

    #[cfg(test)]
    fn for_test_with(
        authority: PasskeyRequestAuthority,
        policy: PlatformAuthorizationRetryPolicy,
        jitter_seed: u64,
    ) -> Self {
        let mut retrier = Self::from_policy(authority, policy, Arc::default());
        retrier.jitter_seed = Some(jitter_seed);
        retrier
    }

    fn cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }

    fn retry_backoff(&self, total_delay: Duration) -> impl backon::Backoff {
        let config = self.policy.config();
        let mut builder = ExponentialBuilder::default()
            .with_min_delay(config.min_delay)
            .with_max_delay(config.max_delay)
            .without_max_times()
            .with_total_delay(Some(total_delay));

        if config.jitter {
            builder = builder.with_jitter();
        }
        #[cfg(test)]
        if let Some(seed) = self.jitter_seed {
            builder = builder.with_jitter_seed(seed);
        }

        builder.build().map(move |delay| delay.min(config.max_delay))
    }

    /// Runs `request` under the permit, retrying transient platform failures
    ///
    /// The outer error is a refusal: nothing was presented. The inner result is the provider's
    /// answer to the last attempt
    async fn retry<T, Request, RequestFuture>(
        &self,
        operation: PasskeyOperation,
        mut request: Request,
    ) -> Result<Result<T, PasskeyError>, PasskeyRequestRefusal>
    where
        Request: FnMut(PasskeyGateGuard) -> RequestFuture,
        RequestFuture: Future<Output = Result<T, PasskeyError>>,
    {
        let started_at = Instant::now();
        let permit = self.authority.permit();
        let result = match permit {
            Ok(permit) => self.retry_under(permit, &mut request).await,
            Err(refusal) => Err(refusal),
        };

        let diagnostic = PasskeyRequestDiagnostic::of(operation, &result, started_at.elapsed());
        match permit {
            Ok(permit) => info!("Passkey request intent={:?} {diagnostic}", permit.intent()),
            Err(_) => info!("Passkey request intent=none {diagnostic}"),
        }

        result
    }

    async fn retry_under<T, Request, RequestFuture>(
        &self,
        permit: &UserPermit,
        request: &mut Request,
    ) -> Result<Result<T, PasskeyError>, PasskeyRequestRefusal>
    where
        Request: FnMut(PasskeyGateGuard) -> RequestFuture,
        RequestFuture: Future<Output = Result<T, PasskeyError>>,
    {
        if self.cancelled() {
            return Ok(Err(PasskeyError::NotCompleted));
        }

        let available_delay = self.deadline.saturating_duration_since(Instant::now());
        let deadline = self.deadline;
        let policy = self.policy;

        // backon builds each attempt after its backoff sleep, so these checks run at the moment
        // of the request: a cancel, an ended operation or a sheet still on screen stops it there
        let attempt = || {
            let ready = if self.cancelled() {
                Err(Attempt::Provider(PasskeyError::NotCompleted))
            } else {
                permit.begin_request().map_err(Attempt::Refused)
            };
            let attempt = ready.map(&mut *request);

            async move { attempt?.await.map_err(Attempt::Provider) }
        };

        let result = attempt
            .retry(self.retry_backoff(available_delay))
            .when(move |error| match error {
                Attempt::Provider(error) => policy.retries(error),
                Attempt::Refused(_) => false,
            })
            .adjust(move |_error, delay| {
                let remaining = deadline.saturating_duration_since(Instant::now());
                delay.filter(|delay| *delay <= remaining)
            })
            .notify(|error, delay| {
                if let Attempt::Provider(error) = error {
                    warn!(
                        "Passkey platform authorization failed before presentation: {error}; retrying in {delay:?}"
                    );
                }
            })
            .await;

        match result {
            Ok(value) => Ok(Ok(value)),
            Err(Attempt::Refused(refusal)) => Err(refusal),
            Err(Attempt::Provider(_)) if self.cancelled() => Ok(Err(PasskeyError::NotCompleted)),
            Err(Attempt::Provider(error)) => Ok(Err(error)),
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the retrier presents requests only under a live permit"
    )]
    pub(crate) async fn discover(
        &self,
        passkey: &PasskeyAccess,
        prf_salt: [u8; 32],
    ) -> Result<Result<DiscoveredPasskeyResult, PasskeyError>, PasskeyRequestRefusal> {
        self.retry(PasskeyOperation::DiscoverAssertion, |gate| {
            let passkey = passkey.clone();

            async move {
                unblock::run_blocking(move || {
                    // the gate stays held until the platform answers
                    let _gate = gate;
                    passkey.discover_and_authenticate_with_prf(
                        PASSKEY_RP_ID.to_string(),
                        prf_salt.to_vec(),
                        random_challenge(),
                    )
                })
                .await
            }
        })
        .await
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the retrier presents requests only under a live permit"
    )]
    pub(crate) async fn create(
        &self,
        passkey: &PasskeyAccess,
        user: PasskeyRegistrationUser,
    ) -> Result<Result<PasskeyRegistrationResult, PasskeyError>, PasskeyRequestRefusal> {
        self.retry(PasskeyOperation::Registration, |gate| {
            let passkey = passkey.clone();
            let user = user.clone();

            async move {
                unblock::run_blocking(move || {
                    // the gate stays held until the platform answers
                    let _gate = gate;
                    passkey.create_passkey(PASSKEY_RP_ID.to_string(), random_challenge(), user)
                })
                .await
            }
        })
        .await
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the retrier presents requests only under a live permit"
    )]
    pub(crate) async fn authenticate(
        &self,
        passkey: &PasskeyAccess,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<Result<Vec<u8>, PasskeyError>, PasskeyRequestRefusal> {
        self.retry(PasskeyOperation::AuthenticateAssertion, |gate| {
            let passkey = passkey.clone();
            let credential_id = credential_id.to_vec();

            async move {
                unblock::run_blocking(move || {
                    // the gate stays held until the platform answers
                    let _gate = gate;
                    passkey.authenticate_with_prf(
                        PASSKEY_RP_ID.to_string(),
                        credential_id,
                        prf_salt.to_vec(),
                        random_challenge(),
                    )
                })
                .await
            }
        })
        .await
    }
}

fn random_challenge() -> Vec<u8> {
    rand::rng().random::<[u8; 32]>().to_vec()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::super::authority::test_support::revocable_permit;
    use super::*;
    use crate::manager::cloud_backup_manager::ops::test_support::async_test_lock;

    #[test]
    fn ios_retries_platform_authorization_failures_for_every_interactive_operation() {
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;

        for operation in [
            PasskeyOperation::Registration,
            PasskeyOperation::DiscoverAssertion,
            PasskeyOperation::AuthenticateAssertion,
        ] {
            assert!(policy.retries(&platform_authorization_error(operation)));
        }
    }

    #[test]
    fn non_ios_retains_only_the_legacy_discovery_retry() {
        let policy = PlatformAuthorizationRetryPolicy::LegacyDiscovery;

        assert!(policy.retries(&platform_authorization_error(PasskeyOperation::DiscoverAssertion)));
        assert!(!policy.retries(&platform_authorization_error(PasskeyOperation::Registration)));
        assert!(
            !policy.retries(&platform_authorization_error(PasskeyOperation::AuthenticateAssertion))
        );
    }

    #[test]
    fn does_not_retry_cancellation_or_post_presentation_failure() {
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;

        assert!(!policy.retries(&PasskeyError::NotCompleted));
        assert!(!policy.retries(&PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
        }));
    }

    #[test]
    fn platform_authorization_retry_budget_extends_beyond_two_seconds() {
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;
        let config = policy.config();
        let retrier = PlatformAuthorizationRetrier::for_test(policy, 7);
        let delays = retrier.retry_backoff(config.total_delay).collect::<Vec<_>>();
        let total_delay = delays.iter().sum::<Duration>();

        assert!(delays[0] >= config.min_delay);
        assert!(total_delay > Duration::from_secs(2));
        assert!(total_delay <= config.total_delay);
        assert!(delays.iter().all(|delay| *delay <= config.max_delay));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ios_retry_recovers_without_retrying_non_transient_failures() {
        let _guard = async_test_lock().lock().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let retry_attempts = Arc::clone(&attempts);
        let retrier = PlatformAuthorizationRetrier::for_test(
            PlatformAuthorizationRetryPolicy::IosInteractive,
            7,
        );
        let result = retrier
            .retry(PasskeyOperation::AuthenticateAssertion, move |_gate| {
                let retry_attempts = Arc::clone(&retry_attempts);

                async move {
                    let attempt = retry_attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt < 2 {
                        Err(platform_authorization_error(PasskeyOperation::AuthenticateAssertion))
                    } else {
                        Ok(())
                    }
                }
            })
            .await;

        assert_eq!(result, Ok(Ok(())));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        for error in [
            PasskeyError::NotCompleted,
            PasskeyError::RequestFailed {
                operation: PasskeyOperation::AuthenticateAssertion,
                reason: PasskeyFailureReason::InvalidResponse,
            },
            PasskeyError::RequestFailed {
                operation: PasskeyOperation::AuthenticateAssertion,
                reason: PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
            },
        ] {
            let attempts = Arc::new(AtomicUsize::new(0));
            let operation_attempts = Arc::clone(&attempts);
            let expected = error.clone();
            let retrier = PlatformAuthorizationRetrier::for_test(
                PlatformAuthorizationRetryPolicy::IosInteractive,
                7,
            );
            let actual = retrier
                .retry(PasskeyOperation::AuthenticateAssertion, move |_gate| {
                    operation_attempts.fetch_add(1, Ordering::SeqCst);
                    let error = error.clone();

                    async move { Err::<(), _>(error) }
                })
                .await;

            assert_eq!(actual, Ok(Err(expected)));
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn sequential_operations_share_one_platform_authorization_retry_deadline() {
        let _guard = async_test_lock().lock().await;
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;
        let retrier = PlatformAuthorizationRetrier::for_test(policy, 7);
        let started_at = Instant::now();
        let first_attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&first_attempts);

        let first_result = retrier
            .retry(PasskeyOperation::AuthenticateAssertion, move |_gate| {
                operation_attempts.fetch_add(1, Ordering::SeqCst);

                async {
                    Err::<(), _>(platform_authorization_error(
                        PasskeyOperation::AuthenticateAssertion,
                    ))
                }
            })
            .await;

        let second_attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&second_attempts);
        let second_result = retrier
            .retry(PasskeyOperation::AuthenticateAssertion, move |_gate| {
                operation_attempts.fetch_add(1, Ordering::SeqCst);

                async {
                    Err::<(), _>(platform_authorization_error(
                        PasskeyOperation::AuthenticateAssertion,
                    ))
                }
            })
            .await;

        assert!(matches!(first_result, Ok(Err(_))));
        assert!(matches!(second_result, Ok(Err(_))));
        assert!(first_attempts.load(Ordering::SeqCst) > 1);
        assert!(second_attempts.load(Ordering::SeqCst) < first_attempts.load(Ordering::SeqCst));
        assert!(Instant::now().duration_since(started_at) <= policy.config().total_delay);
    }

    /// A silent caller can't present a request; it gets `NeedsPasskey` and the
    /// platform is never asked
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn silent_authority_presents_no_request() {
        let _guard = async_test_lock().lock().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let request_attempts = Arc::clone(&attempts);
        let retrier = PlatformAuthorizationRetrier::for_test_with(
            PasskeyRequestAuthority::Silent,
            PlatformAuthorizationRetryPolicy::IosInteractive,
            7,
        );

        let result = retrier
            .retry(PasskeyOperation::AuthenticateAssertion, move |_gate| {
                request_attempts.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await;

        assert_eq!(result, Err(PasskeyRequestRefusal::NeedsPasskey));
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
    }

    /// The permit is checked live before each attempt, so a retry scheduled before
    /// the operation ended presents nothing after it
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_permit_revoked_during_backoff_presents_no_further_request() {
        let _guard = async_test_lock().lock().await;
        let (grant, authority) = revocable_permit(PermitIntent::Verify);
        let grant = Arc::new(parking_lot::Mutex::new(Some(grant)));
        let attempts = Arc::new(AtomicUsize::new(0));
        let request_attempts = Arc::clone(&attempts);
        let retrier = PlatformAuthorizationRetrier::for_test_with(
            authority,
            PlatformAuthorizationRetryPolicy::IosInteractive,
            7,
        );

        let result = retrier
            .retry(PasskeyOperation::AuthenticateAssertion, move |_gate| {
                request_attempts.fetch_add(1, Ordering::SeqCst);

                // the operation ends while this retryable failure waits out its backoff
                grant.lock().take();
                async {
                    Err::<(), _>(platform_authorization_error(
                        PasskeyOperation::AuthenticateAssertion,
                    ))
                }
            })
            .await;

        assert_eq!(result, Err(PasskeyRequestRefusal::Revoked));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    /// A second request while one is open is refused as `Busy` instead of queueing,
    /// and the gate opens again once the first answers
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_second_request_while_one_is_open_is_busy() {
        let _guard = async_test_lock().lock().await;
        let (answer, answered) = tokio::sync::oneshot::channel::<()>();
        let (entered, has_entered) = tokio::sync::oneshot::channel::<()>();
        let first = PlatformAuthorizationRetrier::for_test(
            PlatformAuthorizationRetryPolicy::IosInteractive,
            7,
        );
        let mut entered = Some(entered);
        let mut answered = Some(answered);
        let open = tokio::spawn(async move {
            first
                .retry(PasskeyOperation::AuthenticateAssertion, move |gate| {
                    let entered = entered.take().expect("one attempt");
                    let answered = answered.take().expect("one attempt");

                    async move {
                        let _gate = gate;
                        let _ = entered.send(());
                        let _ = answered.await;
                        Ok::<(), PasskeyError>(())
                    }
                })
                .await
        });
        has_entered.await.expect("the first request reaches the platform");

        let second_attempts = Arc::new(AtomicUsize::new(0));
        let request_attempts = Arc::clone(&second_attempts);
        let second = PlatformAuthorizationRetrier::for_test(
            PlatformAuthorizationRetryPolicy::IosInteractive,
            7,
        );
        let refused = second
            .retry(PasskeyOperation::DiscoverAssertion, move |_gate| {
                request_attempts.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await;

        assert_eq!(refused, Err(PasskeyRequestRefusal::Busy));
        assert_eq!(second_attempts.load(Ordering::SeqCst), 0);

        answer.send(()).expect("the first request is waiting");
        assert_eq!(open.await.expect("first request task"), Ok(Ok(())));

        let after =
            second.retry(PasskeyOperation::DiscoverAssertion, move |_gate| async { Ok(()) }).await;

        assert_eq!(after, Ok(Ok(())));
    }

    fn platform_authorization_error(operation: PasskeyOperation) -> PasskeyError {
        PasskeyError::RequestFailed {
            operation,
            reason: PasskeyFailureReason::PlatformAuthorizationFailed,
        }
    }
}
