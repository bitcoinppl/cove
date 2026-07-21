use std::future::Future;
use std::time::Duration;

use backon::{BackoffBuilder as _, ExponentialBuilder, Retryable as _};
use cove_device::passkey::{
    PasskeyAccess, PasskeyError, PasskeyFailureReason, PasskeyOperation, PasskeyRegistrationResult,
    PasskeyRegistrationUser,
};
use cove_tokio::unblock;
use rand::RngExt as _;
use tokio::time::Instant;
use tracing::warn;

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

        operation_is_in_scope
            && matches!(
                error,
                PasskeyError::RequestFailed {
                    reason: PasskeyFailureReason::PlatformAuthorizationFailed,
                    ..
                }
            )
    }
}

pub(crate) struct PlatformAuthorizationRetrier {
    policy: PlatformAuthorizationRetryPolicy,
    deadline: Instant,
    #[cfg(test)]
    jitter_seed: Option<u64>,
}

impl PlatformAuthorizationRetrier {
    pub(crate) fn new() -> Self {
        Self::from_policy(PlatformAuthorizationRetryPolicy::for_current_platform())
    }

    fn from_policy(policy: PlatformAuthorizationRetryPolicy) -> Self {
        Self {
            policy,
            deadline: Instant::now() + policy.config().total_delay,
            #[cfg(test)]
            jitter_seed: None,
        }
    }

    #[cfg(test)]
    fn for_test(policy: PlatformAuthorizationRetryPolicy, jitter_seed: u64) -> Self {
        let mut retrier = Self::from_policy(policy);
        retrier.jitter_seed = Some(jitter_seed);
        retrier
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

    async fn retry<T, Operation, OperationFuture>(
        &self,
        operation: Operation,
    ) -> Result<T, PasskeyError>
    where
        Operation: FnMut() -> OperationFuture,
        OperationFuture: Future<Output = Result<T, PasskeyError>>,
    {
        let available_delay = self.deadline.saturating_duration_since(Instant::now());
        let deadline = self.deadline;
        let policy = self.policy;

        operation
            .retry(self.retry_backoff(available_delay))
            .when(move |error| policy.retries(error))
            .adjust(move |_error, delay| {
                let remaining = deadline.saturating_duration_since(Instant::now());
                delay.filter(|delay| *delay <= remaining)
            })
            .notify(|error, delay| {
                warn!(
                    "Passkey platform authorization failed before presentation: {error}; retrying in {delay:?}"
                );
            })
            .await
    }

    pub(crate) async fn discover(
        &self,
        passkey: &PasskeyAccess,
        prf_salt: [u8; 32],
    ) -> Result<cove_device::passkey::DiscoveredPasskeyResult, PasskeyError> {
        self.retry(|| {
            let passkey = passkey.clone();

            async move {
                unblock::run_blocking(move || {
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

    pub(crate) async fn create(
        &self,
        passkey: &PasskeyAccess,
        user: PasskeyRegistrationUser,
    ) -> Result<PasskeyRegistrationResult, PasskeyError> {
        self.retry(|| {
            let passkey = passkey.clone();
            let user = user.clone();

            async move {
                unblock::run_blocking(move || {
                    passkey.create_passkey(PASSKEY_RP_ID.to_string(), random_challenge(), user)
                })
                .await
            }
        })
        .await
    }

    pub(crate) async fn authenticate(
        &self,
        passkey: &PasskeyAccess,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<Vec<u8>, PasskeyError> {
        self.retry(|| {
            let passkey = passkey.clone();
            let credential_id = credential_id.to_vec();

            async move {
                unblock::run_blocking(move || {
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

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

        assert!(!policy.retries(&PasskeyError::UserCancelled));
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
        let attempts = Arc::new(AtomicUsize::new(0));
        let retry_attempts = Arc::clone(&attempts);
        let retrier = PlatformAuthorizationRetrier::for_test(
            PlatformAuthorizationRetryPolicy::IosInteractive,
            7,
        );
        let result = retrier
            .retry(move || {
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

        assert_eq!(result, Ok(()));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        for error in [
            PasskeyError::UserCancelled,
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
                .retry(move || {
                    operation_attempts.fetch_add(1, Ordering::SeqCst);
                    let error = error.clone();

                    async move { Err::<(), _>(error) }
                })
                .await;

            assert_eq!(actual, Err(expected));
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn sequential_operations_share_one_platform_authorization_retry_deadline() {
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;
        let retrier = PlatformAuthorizationRetrier::for_test(policy, 7);
        let started_at = Instant::now();
        let first_attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&first_attempts);

        let first_result = retrier
            .retry(move || {
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
            .retry(move || {
                operation_attempts.fetch_add(1, Ordering::SeqCst);

                async {
                    Err::<(), _>(platform_authorization_error(
                        PasskeyOperation::AuthenticateAssertion,
                    ))
                }
            })
            .await;

        assert!(first_result.is_err());
        assert!(second_result.is_err());
        assert!(first_attempts.load(Ordering::SeqCst) > 1);
        assert!(second_attempts.load(Ordering::SeqCst) < first_attempts.load(Ordering::SeqCst));
        assert!(Instant::now().duration_since(started_at) <= policy.config().total_delay);
    }

    fn platform_authorization_error(operation: PasskeyOperation) -> PasskeyError {
        PasskeyError::RequestFailed {
            operation,
            reason: PasskeyFailureReason::PlatformAuthorizationFailed,
        }
    }
}
