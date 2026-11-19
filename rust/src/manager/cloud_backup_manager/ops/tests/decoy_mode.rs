use super::*;
use crate::manager::cloud_backup_manager::screen::CloudBackupSection;
use crate::wallet::metadata::WalletMode as LocalWalletMode;

/// Puts the app in decoy mode for one test and returns it to main mode afterward
///
/// Switches the way the auth manager does: the saved mode first, then the screen
struct DecoyModeGuard<'a>(&'a RustCloudBackupManager);

impl<'a> DecoyModeGuard<'a> {
    fn enter(manager: &'a RustCloudBackupManager) -> Self {
        Database::global().global_config.set_decoy_mode().expect("enter decoy mode");
        manager.wallet_mode_changed(LocalWalletMode::Decoy);
        Self(manager)
    }
}

impl Drop for DecoyModeGuard<'_> {
    fn drop(&mut self) {
        let _ = Database::global().global_config.set_main_mode();
        self.0.wallet_mode_changed(LocalWalletMode::Main);
    }
}

/// The verification presentation the platform was last sent, when the screen has one
fn published_verification_presentation(
    manager: &RustCloudBackupManager,
) -> Option<CloudBackupVerificationPresentation> {
    let CloudBackupScreenBody::Configured(configured) = manager.published_screen().body else {
        return None;
    };

    configured.sections.into_iter().find_map(|section| match section {
        CloudBackupSection::Verification(verification) => Some(verification.view.presentation),
        _ => None,
    })
}

// the decoy PIN must open a Cove that gives no hint of the main wallets' Cloud Backup, so the
// screen keeps its root prompt for main mode only, and entering decoy mode takes it down before
// the supervisor projects again, since the unlock that entered decoy mode may present it first
#[tokio::test(flavor = "current_thread")]
async fn root_prompt_is_published_in_main_mode_only() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    manager.backup_new_wallet(xpub_only_wallet_metadata());
    manager.request_screen_publish();
    supervisor_checkpoint(&manager).await;
    assert_eq!(
        manager.published_snapshot().screen.root_prompt,
        CloudBackupRootPrompt::Verification
    );

    let decoy = DecoyModeGuard::enter(&manager);
    assert_eq!(
        manager.published_snapshot().screen.root_prompt,
        CloudBackupRootPrompt::None,
        "entering decoy mode left the main-mode prompt published until the supervisor ran"
    );

    supervisor_checkpoint(&manager).await;
    assert_eq!(manager.published_snapshot().screen.root_prompt, CloudBackupRootPrompt::None);

    drop(decoy);
    supervisor_checkpoint(&manager).await;
    assert_eq!(
        manager.published_snapshot().screen.root_prompt,
        CloudBackupRootPrompt::Verification
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

// a verification the main-mode prompt started can finish after the decoy PIN unlocks the app,
// and the app's root would announce its result over the decoy wallets
#[tokio::test(flavor = "current_thread")]
async fn root_prompt_verification_results_stay_hidden_in_decoy_mode() {
    use CloudBackupVerificationPresentation as Presentation;
    use CloudBackupVerificationSource::{RootPrompt, Settings};

    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    let results = [
        Presentation::Completed { source: RootPrompt },
        Presentation::Failed { source: RootPrompt, message: "verification failed".into() },
    ];

    for result in results {
        let decoy = DecoyModeGuard::enter(&manager);
        manager.reconcile_verification_presentation(result.clone());
        supervisor_checkpoint(&manager).await;
        assert_eq!(
            published_verification_presentation(&manager),
            Some(Presentation::Hidden { source: Some(RootPrompt) }),
            "decoy mode published {result:?}"
        );

        // the result belongs to main mode, so it shows once the app is back there
        drop(decoy);
        supervisor_checkpoint(&manager).await;
        assert_eq!(published_verification_presentation(&manager), Some(result));
    }

    // the Cloud Backup settings draw their own results, and decoy mode hides those settings
    let _decoy = DecoyModeGuard::enter(&manager);
    let from_settings = Presentation::Completed { source: Settings };
    manager.reconcile_verification_presentation(from_settings.clone());
    supervisor_checkpoint(&manager).await;
    assert_eq!(published_verification_presentation(&manager), Some(from_settings));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}
