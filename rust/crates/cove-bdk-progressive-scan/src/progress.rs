use std::collections::BTreeMap;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScanProgress<K> {
    pub keychain: K,
    pub checked: u32,
    pub gap: u32,
    pub stop_gap: u32,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct KeychainProgress {
    pub checked: u32,
    pub gap: u32,
    pub stop_gap: u32,
}

#[derive(Debug, Clone)]
pub struct ProgressTracker<K> {
    stop_gap: u32,
    keychains: BTreeMap<K, KeychainProgress>,
}

impl<K> ProgressTracker<K>
where
    K: Ord,
{
    pub fn new(stop_gap: usize) -> Self {
        Self { stop_gap: stop_gap.max(1) as u32, keychains: BTreeMap::new() }
    }

    pub fn checked(&mut self, keychain: K, used: bool) -> ScanProgress<K>
    where
        K: Clone,
    {
        let progress = self.keychains.entry(keychain.clone()).or_insert(KeychainProgress {
            checked: 0,
            gap: 0,
            stop_gap: self.stop_gap,
        });
        progress.checked = progress.checked.saturating_add(1);
        progress.gap = if used { 0 } else { progress.gap.saturating_add(1).min(self.stop_gap) };

        ScanProgress {
            keychain,
            checked: progress.checked,
            gap: progress.gap,
            stop_gap: progress.stop_gap,
        }
    }

    pub fn keychains(&self) -> &BTreeMap<K, KeychainProgress> {
        &self.keychains
    }

    pub fn aggregate(&self) -> KeychainProgress {
        self.keychains.values().fold(KeychainProgress::default(), |mut total, progress| {
            total.checked = total.checked.saturating_add(progress.checked);
            total.gap = total.gap.saturating_add(progress.gap);
            total.stop_gap = total.stop_gap.saturating_add(progress.stop_gap);
            total
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{KeychainProgress, ProgressTracker};

    #[test]
    fn progress_increments_checked_for_each_keychain() {
        let mut tracker = ProgressTracker::new(3);

        let external = tracker.checked("external", false);
        let internal = tracker.checked("internal", false);
        let external_again = tracker.checked("external", false);

        assert_eq!(external.checked, 1);
        assert_eq!(internal.checked, 1);
        assert_eq!(external_again.checked, 2);
    }

    #[test]
    fn unused_scripts_increment_gap_until_stop_gap() {
        let mut tracker = ProgressTracker::new(2);

        assert_eq!(tracker.checked("external", false).gap, 1);
        assert_eq!(tracker.checked("external", false).gap, 2);
        assert_eq!(tracker.checked("external", false).gap, 2);
    }

    #[test]
    fn used_scripts_reset_gap() {
        let mut tracker = ProgressTracker::new(3);

        tracker.checked("external", false);
        tracker.checked("external", false);

        let progress = tracker.checked("external", true);

        assert_eq!(progress.checked, 3);
        assert_eq!(progress.gap, 0);
    }

    #[test]
    fn stop_gap_zero_is_treated_as_one() {
        let mut tracker = ProgressTracker::new(0);

        let progress = tracker.checked("external", false);

        assert_eq!(progress.stop_gap, 1);
        assert_eq!(progress.gap, 1);
    }

    #[test]
    fn aggregate_sums_latest_keychain_progress() {
        let mut tracker = ProgressTracker::new(4);

        tracker.checked("external", false);
        tracker.checked("external", false);
        tracker.checked("internal", false);

        assert_eq!(tracker.aggregate(), KeychainProgress { checked: 3, gap: 3, stop_gap: 8 });
    }
}
