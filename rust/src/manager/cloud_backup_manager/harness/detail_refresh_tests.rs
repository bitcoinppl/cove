//! The open detail's inventory refresh: provider signals that arrive while one runs
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PhaseEnd, TestApp, run_case,
};
use crate::manager::cloud_backup_manager::CloudBackupManagerAction;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupProgressSection, CloudBackupScreenBody, CloudBackupSection,
};

/// Provider signals that arrive while the detail's refresh is listing the backup coalesce into one
/// trailing refresh, which runs once that refresh ends and the rate limit allows
#[test]
fn provider_signals_during_a_detail_refresh_run_one_trailing_refresh() {
    let test = concat!(
        module_path!(),
        "::provider_signals_during_a_detail_refresh_run_one_trailing_refresh"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);

        // only a detail refresh lists the inventory snapshot; sync health and supplemental
        // discovery list the plain wallet files, which a provider signal also starts
        let before = app.cloud.count(CloudCall::ListWalletFilesSnapshot);

        let listing = Hold::default();
        app.cloud
            .script_next(CloudCall::ListWalletFilesSnapshot, CloudFault::Hold(listing.clone()));
        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_cannot(&[N::Enable]);
        app.block_on("the detail refresh's snapshot listing to be held", listing.entered());

        app.event("the provider reports changes twice while the listing is held", |app| {
            app.manager().cloud_storage_did_change();
            app.manager().cloud_storage_did_change();
        })
        .then_cannot(&[N::Enable]);
        assert_eq!(
            app.cloud.count(CloudCall::ListWalletFilesSnapshot),
            before + 1,
            "{}",
            app.describe()
        );

        app.event("the held listing answers", |_| listing.release()).then_cannot(&[N::Enable]);
        let after_first = app.cloud.count(CloudCall::ListWalletFilesSnapshot);

        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
        let trailing = app.cloud.count(CloudCall::ListWalletFilesSnapshot) - after_first;
        assert_eq!(trailing, 1, "expected one trailing refresh\n{}", app.describe());

        app.quiesce().then_can(&[N::TurnOff]);
        assert_eq!(
            app.cloud.count(CloudCall::ListWalletFilesSnapshot),
            after_first + 1,
            "the trailing refresh was followed by another\n{}",
            app.describe()
        );
        let CloudBackupScreenBody::Configured(configured) = app.manager().published_screen().body
        else {
            panic!("the detail is not configured\n{}", app.describe());
        };
        assert!(
            !configured.sections.iter().any(|section| matches!(
                section,
                CloudBackupSection::Progress(CloudBackupProgressSection::InventoryChecking)
            )),
            "the detail is still checking\n{}",
            app.describe()
        );
    });
}
