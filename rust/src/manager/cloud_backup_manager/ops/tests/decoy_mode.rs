use super::*;

/// Puts the app in decoy mode for one test and returns it to main mode afterward
struct DecoyModeGuard;

impl DecoyModeGuard {
    fn enter() -> Self {
        Database::global().global_config.set_decoy_mode().expect("enter decoy mode");
        Self
    }
}

impl Drop for DecoyModeGuard {
    fn drop(&mut self) {
        let _ = Database::global().global_config.set_main_mode();
    }
}

// the decoy PIN must open a Cove that gives no hint of the main wallets' Cloud Backup, so the
// screen keeps its root prompt for main mode only
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

    let decoy = DecoyModeGuard::enter();
    manager.request_screen_publish();
    supervisor_checkpoint(&manager).await;
    assert_eq!(manager.published_snapshot().screen.root_prompt, CloudBackupRootPrompt::None);

    drop(decoy);
    manager.request_screen_publish();
    supervisor_checkpoint(&manager).await;
    assert_eq!(
        manager.published_snapshot().screen.root_prompt,
        CloudBackupRootPrompt::Verification
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}
