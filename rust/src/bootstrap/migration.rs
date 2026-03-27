use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

use parking_lot::Mutex;
use tracing::info;

static ACTIVE_MIGRATION: Mutex<Option<Arc<Migration>>> = Mutex::new(None);

#[derive(uniffi::Object)]
pub struct Migration {
    current: AtomicU32,
    total: AtomicU32,
    cancelled: Arc<AtomicBool>,
}

#[uniffi::export]
impl Migration {
    pub fn progress(&self) -> MigrationProgress {
        let total = self.total.load(Ordering::Acquire);
        MigrationProgress { current: self.current.load(Ordering::Acquire).min(total), total }
    }

    /// Cancel the migration, equivalent to calling `cancel_bootstrap()`
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        info!("Migration cancellation requested");
    }
}

impl Migration {
    pub(crate) fn new(total: u32, cancelled: Arc<AtomicBool>) -> Self {
        Self { current: AtomicU32::new(0), total: AtomicU32::new(total), cancelled }
    }

    pub(crate) fn tick(&self) {
        let current = self.current.load(Ordering::Acquire);
        let total = self.total.load(Ordering::Acquire);

        // auto-expand total if count functions undercounted due to I/O errors
        if current >= total {
            self.total.fetch_add(1, Ordering::Release);
        }

        self.current.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(uniffi::Record)]
pub struct MigrationProgress {
    pub current: u32,
    pub total: u32,
}

/// Returns the active migration object if one has been registered,
/// used by the frontend to poll progress
#[uniffi::export]
pub fn active_migration() -> Option<Arc<Migration>> {
    ACTIVE_MIGRATION.lock().clone()
}

/// Register or clear the active migration (for frontend progress polling)
pub(crate) fn set_active_migration(migration: Option<Arc<Migration>>) {
    *ACTIVE_MIGRATION.lock() = migration;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn test_migration(total: u32) -> Migration {
        Migration::new(total, Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn tick_increments_current() {
        let m = test_migration(5);
        assert_eq!(m.progress().current, 0);
        m.tick();
        assert_eq!(m.progress().current, 1);
        m.tick();
        assert_eq!(m.progress().current, 2);
    }

    #[test]
    fn tick_auto_expands_total() {
        let m = test_migration(2);
        m.tick();
        m.tick();
        assert_eq!(m.progress().total, 2);

        // tick when current >= total should expand
        m.tick();
        assert_eq!(m.progress().total, 3);
        assert_eq!(m.progress().current, 3);
    }

    #[test]
    fn progress_clamps_current_to_total() {
        let m = test_migration(3);
        m.tick();
        m.tick();
        m.tick();

        let p = m.progress();
        assert!(
            p.current <= p.total,
            "current ({}) should not exceed total ({})",
            p.current,
            p.total
        );
    }
}
