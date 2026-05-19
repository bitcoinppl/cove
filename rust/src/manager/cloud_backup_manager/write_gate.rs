use std::future::Future;

use parking_lot::RwLock;

use super::CloudBackupError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudWriteBlocker {
    Disabling { operation_id: u64 },
}

#[derive(Debug, Default)]
pub(crate) struct CloudWriteGate {
    active_blocker: RwLock<Option<CloudWriteBlocker>>,
    write_lock: tokio::sync::Mutex<()>,
}

impl CloudWriteGate {
    pub(crate) fn block(&self, blocker: CloudWriteBlocker) {
        *self.active_blocker.write() = Some(blocker);
    }

    pub(crate) fn unblock(&self, blocker: CloudWriteBlocker) {
        let mut active_blocker = self.active_blocker.write();
        if *active_blocker == Some(blocker) {
            *active_blocker = None;
        }
    }

    pub(crate) fn active_blocker(&self) -> Option<CloudWriteBlocker> {
        *self.active_blocker.read()
    }

    fn writes_blocked(&self) -> bool {
        self.active_blocker().is_some()
    }

    pub(crate) async fn run_allowed_write<T>(
        &self,
        operation: impl Future<Output = Result<T, CloudBackupError>>,
        writes_blocked_by_persisted_state: impl Fn() -> bool,
    ) -> Result<T, CloudBackupError> {
        let _guard = self.write_lock.lock().await;
        self.ensure_writes_allowed(writes_blocked_by_persisted_state())?;
        operation.await
    }

    pub(crate) async fn run_exclusive_write<T>(&self, operation: impl Future<Output = T>) -> T {
        let _guard = self.write_lock.lock().await;
        operation.await
    }

    pub(crate) fn ensure_writes_allowed(
        &self,
        writes_blocked_by_persisted_state: bool,
    ) -> Result<(), CloudBackupError> {
        if self.writes_blocked() || writes_blocked_by_persisted_state {
            return Err(CloudBackupError::Deferred(
                "cloud backup writes are paused while disabling cloud backup".into(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn run_allowed_write_preserves_success_when_writes_block_after_operation() {
        let gate = CloudWriteGate::default();
        let blocked = AtomicBool::new(false);

        let result = gate
            .run_allowed_write(
                async {
                    blocked.store(true, Ordering::Relaxed);
                    Ok::<_, CloudBackupError>(42)
                },
                || blocked.load(Ordering::Relaxed),
            )
            .await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_allowed_write_preserves_operation_error_when_writes_block_after_operation() {
        let gate = CloudWriteGate::default();
        let blocked = AtomicBool::new(false);

        let result = gate
            .run_allowed_write(
                async {
                    blocked.store(true, Ordering::Relaxed);
                    Err::<(), _>(CloudBackupError::Internal("operation failed".into()))
                },
                || blocked.load(Ordering::Relaxed),
            )
            .await;

        assert!(
            matches!(result, Err(CloudBackupError::Internal(message)) if message == "operation failed")
        );
    }
}
