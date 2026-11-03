//! App lock ownership
//!
//! The app can be locked for several independent reasons at once. Each lock records
//! who took it, so a caller can only release the lock it owns, and locks that guard
//! against someone else holding the device only clear after real authentication

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use crate::auth::AuthType;

/// How long after taking it a send confirmation lock can be dropped by leaving the screen
///
/// Leaving the confirmation screen right away means the user backed out before being
/// asked to authenticate, so the lock no longer protects anything. After that the lock
/// stays until the user authenticates
const SEND_CONFIRMATION_RELEASE_WINDOW: Duration = Duration::from_secs(5);

/// How quickly the app must return to the foreground to skip authentication when only
/// a PIN is set and no decoy PIN exists
const PIN_ONLY_FOREGROUND_GRACE: Duration = Duration::from_secs(2);

/// How quickly the app must return to the foreground to skip authentication otherwise
const FOREGROUND_GRACE: Duration = Duration::from_secs(1);

/// Why the app is being locked until the user authenticates
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AppLockReason {
    /// The app left the foreground
    Background,
    /// A screen needs the user to authenticate again before showing or using secrets
    Reauthentication,
}

/// Proof that a send confirmation screen took a lock, needed to release that lock
///
/// Frontends keep this opaque and hand it back when the screen goes away
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Record)]
pub struct SendConfirmationLock {
    id: u64,
}

/// Lock that only authentication clears
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthenticationLock {
    /// When the app went to the background, only set while leaving the foreground is the
    /// sole reason for this lock, so a quick return can skip authentication
    background_since: Option<Instant>,
}

/// Every lock currently held on the app
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppLock {
    authentication: Option<AuthenticationLock>,
    send_confirmations: BTreeMap<u64, Instant>,
    next_send_confirmation_id: u64,
}

impl AppLock {
    /// The app starts locked, and a cold start never qualifies for the foreground grace
    pub(crate) fn locked_at_startup() -> Self {
        Self {
            authentication: Some(AuthenticationLock { background_since: None }),
            send_confirmations: BTreeMap::new(),
            next_send_confirmation_id: 0,
        }
    }

    pub(crate) fn is_locked(&self) -> bool {
        self.authentication.is_some() || !self.send_confirmations.is_empty()
    }

    /// Lock until the user authenticates
    pub(crate) fn lock(&mut self, reason: AppLockReason, now: Instant) {
        match (reason, self.authentication.as_mut()) {
            (AppLockReason::Background, None) => {
                self.authentication = Some(AuthenticationLock { background_since: Some(now) });
            }

            // keep the earliest background time, and never make an existing lock
            // eligible for the foreground grace
            (AppLockReason::Background, Some(_)) => {}

            (AppLockReason::Reauthentication, _) => {
                self.authentication = Some(AuthenticationLock { background_since: None });
            }
        }
    }

    /// Take a lock owned by one send confirmation screen
    pub(crate) fn lock_for_send_confirmation(&mut self, now: Instant) -> SendConfirmationLock {
        let id = self.next_send_confirmation_id;
        self.next_send_confirmation_id = id.wrapping_add(1);
        self.send_confirmations.insert(id, now);

        SendConfirmationLock { id }
    }

    /// Release a send confirmation lock when its screen goes away
    ///
    /// Only removes the lock this token owns, and only inside the release window, so any
    /// other lock still needs authentication
    pub(crate) fn release_send_confirmation(&mut self, lock: SendConfirmationLock, now: Instant) {
        let Some(locked_at) = self.send_confirmations.get(&lock.id) else { return };

        if now.saturating_duration_since(*locked_at) < SEND_CONFIRMATION_RELEASE_WINDOW {
            self.send_confirmations.remove(&lock.id);
        }
    }

    /// Release the background lock if the app came back within the grace period
    ///
    /// Locks taken for any other reason stay in place
    pub(crate) fn release_background_on_foreground(
        &mut self,
        auth_type: AuthType,
        is_decoy_pin_enabled: bool,
        now: Instant,
    ) {
        let Some(AuthenticationLock { background_since: Some(since) }) = self.authentication else {
            return;
        };

        let grace = match auth_type {
            AuthType::Pin if !is_decoy_pin_enabled => PIN_ONLY_FOREGROUND_GRACE,
            _ => FOREGROUND_GRACE,
        };

        if now.saturating_duration_since(since) < grace {
            self.authentication = None;
        }
    }

    /// Clear every lock after the user authenticated
    pub(crate) fn unlock_authenticated(&mut self) {
        self.authentication = None;
        self.send_confirmations.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> (AppLock, Instant) {
        let mut lock = AppLock::locked_at_startup();
        lock.unlock_authenticated();

        (lock, Instant::now())
    }

    #[test]
    fn send_confirmation_release_after_window_keeps_lock() {
        let (mut lock, now) = start();
        let send = lock.lock_for_send_confirmation(now);

        lock.release_send_confirmation(send, now + SEND_CONFIRMATION_RELEASE_WINDOW);

        assert!(lock.is_locked());
    }

    #[test]
    fn send_confirmation_release_only_removes_its_own_lock() {
        let (mut lock, now) = start();
        let first = lock.lock_for_send_confirmation(now);
        let second = lock.lock_for_send_confirmation(now);

        lock.release_send_confirmation(first, now + Duration::from_secs(1));
        assert!(lock.is_locked());

        lock.release_send_confirmation(second, now + Duration::from_secs(1));
        assert!(!lock.is_locked());
    }

    #[test]
    fn foreground_grace_never_releases_reauthentication_lock() {
        let (mut lock, now) = start();
        lock.lock(AppLockReason::Reauthentication, now);
        lock.lock(AppLockReason::Background, now);

        lock.release_background_on_foreground(AuthType::Pin, false, now);

        assert!(lock.is_locked());
    }

    #[test]
    fn foreground_grace_depends_on_auth_type() {
        let (mut lock, now) = start();
        let back = now + Duration::from_millis(1500);

        lock.lock(AppLockReason::Background, now);
        lock.release_background_on_foreground(AuthType::Both, false, back);
        assert!(lock.is_locked());

        lock.release_background_on_foreground(AuthType::Pin, true, back);
        assert!(lock.is_locked());

        lock.release_background_on_foreground(AuthType::Pin, false, back);
        assert!(!lock.is_locked());
    }

    #[test]
    fn cold_start_needs_authentication() {
        let mut lock = AppLock::locked_at_startup();
        lock.release_background_on_foreground(AuthType::Pin, false, Instant::now());

        assert!(lock.is_locked());
    }
}
