//! The rule the platform contract tests hold the screen to: every action the screen offers has a
//! control on the surface that draws it, and that surface's content allows the control
//!
//! The screen names each control and its surface ([`CloudBackupScreen::drawn_controls`]), and the
//! platforms draw exactly those. [`misplaced_controls`] is written from the platforms' views rather
//! than from the projection: it holds each control to the content its surface shows, so a
//! projection that names a rebuild on a backup that verifies, or Sync Now with nothing to sync,
//! fails here. The fixture test applies these to every named fixture, and harness invariant 4a
//! applies [`undrawn_actions`] to every screen the harness publishes
#![cfg(test)]

use std::collections::HashSet;

use cove_device::cloud_storage::CloudSyncHealth;

use super::types::{
    CloudBackupConfiguredScreen, CloudBackupControlSurface, CloudBackupDrawnControl,
    CloudBackupFinishingDisableStage, CloudBackupInventoryControl, CloudBackupOtherBackupsControl,
    CloudBackupOtherBackupsSection, CloudBackupProgressSection, CloudBackupRecoveryKind,
    CloudBackupScreen, CloudBackupScreenAction, CloudBackupScreenBody, CloudBackupSection,
    CloudBackupTurnOffFailure, CloudBackupVerificationControl, CloudBackupVerificationSection,
    CloudBackupWalletsControl, CloudBackupWalletsSection,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupInventoryIncompleteReason, CloudBackupOtherBackupsState,
    CloudBackupPasskeyChoiceIntent, CloudBackupRootPrompt, CloudBackupSyncState,
    CloudBackupVerificationState, DeepVerificationFailure,
};

/// The actions `screen` offers with no well-placed control: nothing draws them, or only a
/// control its surface's content does not allow
pub(crate) fn undrawn_actions(screen: &CloudBackupScreen) -> Vec<CloudBackupScreenAction> {
    let mut placed = screen.drawn_controls();
    for misplaced in misplaced_controls(screen) {
        if let Some(index) = placed.iter().position(|control| *control == misplaced) {
            placed.remove(index);
        }
    }

    screen
        .actions
        .entries
        .iter()
        .map(|entry| entry.action)
        .filter(|action| !placed.iter().any(|control| control.action == *action))
        .collect()
}

/// The section kinds a configured `screen` lists more than once
///
/// The platforms key sections by kind (iOS draws them in a `ForEach` over `\.kind`), so a kind
/// listed twice gives two sections one identity
pub(crate) fn repeated_section_kinds(screen: &CloudBackupScreen) -> Vec<&'static str> {
    let CloudBackupScreenBody::Configured(configured) = &screen.body else { return Vec::new() };

    let mut seen = HashSet::new();
    configured.sections.iter().map(section_kind).filter(|kind| !seen.insert(*kind)).collect()
}

/// A section's identity as the platforms key it, matching iOS's `CloudBackupSection.kind`
pub(crate) fn section_kind(section: &CloudBackupSection) -> &'static str {
    match section {
        CloudBackupSection::PasskeyCard(_) => "passkeyCard",
        CloudBackupSection::Progress(_) => "progress",
        CloudBackupSection::Verification(_) => "verification",
        CloudBackupSection::Wallets(_) => "wallets",
        CloudBackupSection::NotOnDevice(_) => "notOnDevice",
        CloudBackupSection::ExtraCopies(_) => "extraCopies",
        CloudBackupSection::OtherBackups(_) => "otherBackups",
        CloudBackupSection::TurnOff => "turnOff",
    }
}

/// The controls `screen` draws whose action has no entry in its table, so no platform can say
/// whether it is available
pub(crate) fn unlisted_controls(screen: &CloudBackupScreen) -> Vec<CloudBackupDrawnControl> {
    screen
        .drawn_controls()
        .into_iter()
        .filter(|control| screen.actions.availability(control.action).is_none())
        .collect()
}

/// The controls `screen` names on a surface whose content does not allow them
pub(crate) fn misplaced_controls(screen: &CloudBackupScreen) -> Vec<CloudBackupDrawnControl> {
    let mut misplaced = misplaced_prompt_controls(screen);
    misplaced.extend(match &screen.body {
        CloudBackupScreenBody::Configured(configured) => misplaced_configured_controls(configured),
        CloudBackupScreenBody::FinishingDisable(finishing) => {
            misplaced_finishing_disable_controls(screen, finishing.stage)
        }
        _ => Vec::new(),
    });

    misplaced
}

/// Each prompt choice belongs with the prompt that draws it: Use Existing on every Enable prompt,
/// Create New only on one that can start a new backup, the repair on the repair prompt and Verify
/// on the verification prompt
fn misplaced_prompt_controls(screen: &CloudBackupScreen) -> Vec<CloudBackupDrawnControl> {
    use CloudBackupPasskeyChoiceIntent as Choice;
    use CloudBackupRootPrompt as Prompt;
    use CloudBackupScreenAction as A;

    let prompt = &screen.root_prompt;
    let allowed = |action: CloudBackupScreenAction| match action {
        A::AcceptEnablePromptUseExisting => matches!(
            prompt,
            Prompt::ExistingBackupFound(..)
                | Prompt::PasskeyChoice(Choice::Enable(..) | Choice::EnableExistingPasskeyOnly(..))
        ),
        A::AcceptEnablePromptCreateNew => matches!(
            prompt,
            Prompt::ExistingBackupFound(..) | Prompt::PasskeyChoice(Choice::Enable(..))
        ),
        A::RepairPasskey => *prompt == Prompt::PasskeyChoice(Choice::RepairPasskey),
        A::Verify => *prompt == Prompt::Verification,
        _ => false,
    };

    screen
        .drawn_controls()
        .into_iter()
        .filter(|control| {
            control.surface == CloudBackupControlSurface::RootPrompt && !allowed(control.action)
        })
        .collect()
}

/// Android's finishing actions draw Reconnect only once the provider stopped the last run for
/// consent
fn misplaced_finishing_disable_controls(
    screen: &CloudBackupScreen,
    stage: CloudBackupFinishingDisableStage,
) -> Vec<CloudBackupDrawnControl> {
    let reconnect = stage
        == CloudBackupFinishingDisableStage::Failed {
            reason: CloudBackupTurnOffFailure::AuthorizationRequired,
        };

    screen
        .drawn_controls()
        .into_iter()
        .filter(|control| {
            control.surface == CloudBackupControlSurface::Body
                && control.action == CloudBackupScreenAction::ReconnectDrive
                && !reconnect
        })
        .collect()
}

/// Each configured section's controls held to the content that section shows
fn misplaced_configured_controls(
    configured: &CloudBackupConfiguredScreen,
) -> Vec<CloudBackupDrawnControl> {
    let mut misplaced = Vec::new();
    for section in &configured.sections {
        match section {
            CloudBackupSection::Verification(verification) => {
                misplaced.extend(misplaced_verification_controls(verification, configured));
            }
            CloudBackupSection::Wallets(wallets) => {
                misplaced.extend(misplaced_wallets_controls(wallets));
            }
            CloudBackupSection::Progress(CloudBackupProgressSection::InventoryIncomplete {
                reason,
                controls,
            }) => {
                misplaced.extend(misplaced_inventory_controls(*reason, controls, configured));
            }
            CloudBackupSection::OtherBackups(other) => {
                misplaced.extend(misplaced_other_backups_controls(other));
            }
            CloudBackupSection::PasskeyCard(_)
            | CloudBackupSection::Progress(_)
            | CloudBackupSection::NotOnDevice(_)
            | CloudBackupSection::ExtraCopies(_)
            | CloudBackupSection::TurnOff => {}
        }
    }

    misplaced
}

/// Reconnect belongs with an inventory that stopped because Drive asked to be authorized again
fn misplaced_inventory_controls(
    reason: CloudBackupInventoryIncompleteReason,
    controls: &[CloudBackupInventoryControl],
    configured: &CloudBackupConfiguredScreen,
) -> Vec<CloudBackupDrawnControl> {
    let reconnect = reason == CloudBackupInventoryIncompleteReason::AuthorizationRequired
        || drive_authorization_required(configured);

    controls
        .iter()
        .filter(|control| match control {
            CloudBackupInventoryControl::CheckAgain => false,
            CloudBackupInventoryControl::ReconnectDrive => !reconnect,
        })
        .map(|control| drawn(CloudBackupControlSurface::InventoryIncomplete, control.action()))
        .collect()
}

/// Each verification control belongs with the state that offers it: a rebuild with the failure
/// that names it, the undecryptable cleanup with a report that counts some, and the upload
/// controls with an upload waiting to be confirmed
fn misplaced_verification_controls(
    section: &CloudBackupVerificationSection,
    configured: &CloudBackupConfiguredScreen,
) -> Vec<CloudBackupDrawnControl> {
    use CloudBackupVerificationControl as C;
    use CloudBackupVerificationState as S;

    let state = &section.view.state;
    let allowed = |control: &C| match control {
        C::Verify => !matches!(
            state,
            S::Running
                | S::AwaitingUploadConfirmation
                | S::Failed(DeepVerificationFailure::UnsupportedVersion { .. })
        ),
        C::Recover { kind: CloudBackupRecoveryKind::RecreateManifest } => {
            matches!(state, S::Failed(DeepVerificationFailure::RecreateManifest { .. }))
        }
        C::Recover { kind: CloudBackupRecoveryKind::ReinitializeBackup } => {
            matches!(state, S::Failed(DeepVerificationFailure::ReinitializeBackup { .. }))
        }
        C::DeleteUnreadableBackups => matches!(
            state,
            S::NeedsAttention { report, .. } if report.wallet_issues.decryption_failed > 0
        ),
        C::RetryUpload => {
            matches!(state, S::AwaitingUploadConfirmation) && upload_failed(configured)
        }
        C::ReconnectDrive => {
            matches!(state, S::AwaitingUploadConfirmation)
                && drive_authorization_required(configured)
        }
    };

    section
        .controls
        .iter()
        .filter(|control| !allowed(control))
        .map(|control| drawn(CloudBackupControlSurface::Verification, control.action()))
        .collect()
}

/// Sync Now sits under the wallets that need it, so it needs at least one
fn misplaced_wallets_controls(section: &CloudBackupWalletsSection) -> Vec<CloudBackupDrawnControl> {
    section
        .controls
        .iter()
        .filter(|control| match control {
            CloudBackupWalletsControl::SyncNow => section.needs_sync.is_empty(),
        })
        .map(|control| drawn(CloudBackupControlSurface::Wallets, control.action()))
        .collect()
}

/// Recover and Delete act on the other backups a loaded list shows, so it must show some, and
/// Check Again answers a check that failed
fn misplaced_other_backups_controls(
    section: &CloudBackupOtherBackupsSection,
) -> Vec<CloudBackupDrawnControl> {
    use CloudBackupOtherBackupsControl as C;

    let shows_backups = matches!(
        &section.state,
        CloudBackupOtherBackupsState::Loaded { summary } if summary.namespace_count > 0
    );
    let check_failed = matches!(section.state, CloudBackupOtherBackupsState::LoadFailed { .. });

    section
        .controls
        .iter()
        .filter(|control| match control {
            C::Recover | C::Delete => !shows_backups,
            C::CheckAgain => !check_failed,
        })
        .map(|control| drawn(CloudBackupControlSurface::OtherBackups, control.action()))
        .collect()
}

fn upload_failed(configured: &CloudBackupConfiguredScreen) -> bool {
    configured.sections.iter().any(|section| {
        matches!(
            section,
            CloudBackupSection::Wallets(wallets)
                if matches!(wallets.sync, CloudBackupSyncState::Failed(_))
        )
    })
}

fn drive_authorization_required(configured: &CloudBackupConfiguredScreen) -> bool {
    matches!(configured.sync_health, CloudSyncHealth::AuthorizationRequired(_))
}

fn drawn(
    surface: CloudBackupControlSurface,
    action: CloudBackupScreenAction,
) -> CloudBackupDrawnControl {
    CloudBackupDrawnControl { action, surface }
}

mod tests {
    use super::super::fixtures::cloud_backup_screen_fixtures;
    use super::super::types::{CloudBackupActionAvailability, CloudBackupActionEntry};
    use super::*;

    use CloudBackupScreenAction as A;
    use CloudBackupScreenBody as Body;

    #[test]
    fn every_fixture_draws_each_control_it_offers_where_its_content_allows() {
        let fixtures = cloud_backup_screen_fixtures();
        assert!(!fixtures.is_empty(), "debug builds name their fixtures");

        for fixture in &fixtures {
            let screen = &fixture.screen;
            let undrawn = undrawn_actions(screen);
            assert!(undrawn.is_empty(), "{} offers {undrawn:?} with no control", fixture.name);

            let unlisted = unlisted_controls(screen);
            assert!(unlisted.is_empty(), "{} draws {unlisted:?} with no entry", fixture.name);

            let misplaced = misplaced_controls(screen);
            assert!(misplaced.is_empty(), "{} misplaces {misplaced:?}", fixture.name);

            let repeated = repeated_section_kinds(screen);
            assert!(repeated.is_empty(), "{} lists {repeated:?} more than once", fixture.name);
        }
    }

    /// An entry with no control is caught, and so is a control with no entry
    #[test]
    fn the_table_and_the_drawn_controls_name_the_same_actions() {
        let mut clean = named("configured-clean");
        clean.actions.entries.push(CloudBackupActionEntry {
            action: A::RefreshDetail,
            availability: CloudBackupActionAvailability::Available,
        });

        assert_eq!(undrawn_actions(&clean), vec![A::RefreshDetail]);

        let mut clean = named("configured-clean");
        clean.actions.entries.retain(|entry| entry.action != A::Verify);
        let unlisted = unlisted_controls(&clean);
        assert_eq!(unlisted, vec![drawn(CloudBackupControlSurface::Verification, A::Verify)]);
    }

    /// A rebuild drawn on a backup that verifies, or Sync Now with every wallet up to date, has
    /// a surface but not one whose content allows it, so it counts as undrawn
    #[test]
    fn a_control_on_a_surface_whose_content_does_not_allow_it_is_caught() {
        let recreate = CloudBackupVerificationControl::Recover {
            kind: CloudBackupRecoveryKind::RecreateManifest,
        };

        let mut clean = named("configured-clean");
        verification_section(&mut clean).controls.push(recreate);
        clean.actions.entries.push(CloudBackupActionEntry {
            action: A::RecreateManifest,
            availability: CloudBackupActionAvailability::Available,
        });

        let surface = CloudBackupControlSurface::Verification;
        assert_eq!(misplaced_controls(&clean), vec![drawn(surface, A::RecreateManifest)]);
        assert_eq!(undrawn_actions(&clean), vec![A::RecreateManifest]);

        let mut clean = named("configured-clean");
        wallets_section(&mut clean).controls.push(CloudBackupWalletsControl::SyncNow);
        clean.actions.entries.push(CloudBackupActionEntry {
            action: A::SyncUnsynced,
            availability: CloudBackupActionAvailability::Available,
        });

        let surface = CloudBackupControlSurface::Wallets;
        assert_eq!(misplaced_controls(&clean), vec![drawn(surface, A::SyncUnsynced)]);
        assert_eq!(undrawn_actions(&clean), vec![A::SyncUnsynced]);
    }

    /// A backup that verifies and has nothing to sync offers Verify and Turn Off, and Android's
    /// account switch; no rebuild and no Sync Now
    #[test]
    fn a_clean_backup_offers_no_rebuild_and_no_sync() {
        let clean = named("configured-clean");
        let mut actions: Vec<_> = clean.actions.entries.iter().map(|entry| entry.action).collect();
        actions.sort_by_key(|action| format!("{action:?}"));

        assert_eq!(actions, vec![A::SwitchDriveAccount, A::TurnOff, A::Verify]);
    }

    /// A failure names the one recovery it allows; a backup only a newer Cove can read offers no
    /// Verify, because verifying again can't read it either
    #[test]
    fn each_verification_failure_offers_only_its_recovery() {
        let cases = [
            ("configured-verification-recreate-manifest", Some(A::RecreateManifest)),
            ("configured-verification-reinitialize-backup", Some(A::ReinitializeBackup)),
            ("configured-verification-unsupported-version", None),
        ];

        for (name, recovery) in cases {
            let screen = named(name);
            let offered = |action| screen.actions.availability(action).is_some();
            let rebuilds: Vec<_> = [A::RecreateManifest, A::ReinitializeBackup]
                .into_iter()
                .filter(|action| offered(*action))
                .collect();

            assert_eq!(rebuilds, recovery.into_iter().collect::<Vec<_>>(), "{name}");
            assert!(!offered(A::Verify), "{name} offers Verify");
        }

        let empty = named("configured-empty-inventory");
        assert!(empty.actions.availability(A::SyncUnsynced).is_none());
        assert!(empty.actions.availability(A::Verify).is_some());
    }

    /// A running Verify takes the one progress section from an incomplete inventory check, and a
    /// second progress section is caught
    #[test]
    fn a_running_verification_holds_the_only_progress_section() {
        let mut screen = named("configured-verifying-inventory-incomplete");
        let Body::Configured(configured) = &screen.body else { panic!("a configured screen") };
        let kinds: Vec<_> = configured.sections.iter().map(section_kind).collect();

        assert_eq!(
            kinds,
            ["progress", "verification", "wallets", "notOnDevice", "extraCopies", "turnOff"]
        );
        assert!(matches!(
            &configured.sections[0],
            CloudBackupSection::Progress(CloudBackupProgressSection::VerificationInline(view))
                if view.state == CloudBackupVerificationState::Running
        ));

        let Body::Configured(configured) = &mut screen.body else { unreachable!() };
        configured
            .sections
            .insert(1, CloudBackupSection::Progress(CloudBackupProgressSection::InventoryChecking));
        assert_eq!(repeated_section_kinds(&screen), ["progress"]);
    }

    fn named(name: &str) -> CloudBackupScreen {
        cloud_backup_screen_fixtures()
            .into_iter()
            .find(|fixture| fixture.name.ends_with(&format!(":{name}")))
            .map(|fixture| fixture.screen)
            .unwrap_or_else(|| panic!("the {name} fixture"))
    }

    fn verification_section(screen: &mut CloudBackupScreen) -> &mut CloudBackupVerificationSection {
        let Body::Configured(configured) = &mut screen.body else { panic!("a configured screen") };
        configured
            .sections
            .iter_mut()
            .find_map(|section| match section {
                CloudBackupSection::Verification(verification) => Some(verification),
                _ => None,
            })
            .expect("a verification section")
    }

    fn wallets_section(screen: &mut CloudBackupScreen) -> &mut CloudBackupWalletsSection {
        let Body::Configured(configured) = &mut screen.body else { panic!("a configured screen") };
        configured
            .sections
            .iter_mut()
            .find_map(|section| match section {
                CloudBackupSection::Wallets(wallets) => Some(wallets),
                _ => None,
            })
            .expect("a wallets section")
    }
    /// A root prompt draws its choices on its own surface, each bound to its own entry, so a
    /// prompt whose choice has no entry is caught like any other control
    #[test]
    fn a_root_prompt_draws_each_of_its_choices_on_its_surface() {
        let prompt_actions = |screen: &CloudBackupScreen| -> Vec<A> {
            screen
                .drawn_controls()
                .into_iter()
                .filter(|control| control.surface == CloudBackupControlSurface::RootPrompt)
                .map(|control| control.action)
                .collect()
        };

        let existing = named("enabling-existing-backup-prompt");
        let both = vec![A::AcceptEnablePromptUseExisting, A::AcceptEnablePromptCreateNew];
        assert_eq!(prompt_actions(&existing), both);
        assert_eq!(
            prompt_actions(&named("enabling-passkey-choice-existing-only")),
            vec![A::AcceptEnablePromptUseExisting]
        );
        assert_eq!(prompt_actions(&named("configured-verification-prompt")), vec![A::Verify]);
        assert_eq!(prompt_actions(&named("configured")), Vec::<A>::new());

        let mut existing = existing;
        existing.actions.entries.retain(|entry| entry.action != A::AcceptEnablePromptCreateNew);
        let surface = CloudBackupControlSurface::RootPrompt;
        assert_eq!(
            unlisted_controls(&existing),
            vec![drawn(surface, A::AcceptEnablePromptCreateNew)]
        );
    }
}
