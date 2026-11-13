//! The accessibility identifiers both apps give Cloud Backup's controls and surfaces
//!
//! This is the one mapping. Swift reads it through [`CloudBackupUiElement::accessibility_identifier`].
//! Kotlin keeps a copy, because its previews and screenshot tests run without the Rust library, and
//! its contract test checks that copy against every element [`cloud_backup_ui_elements`] lists. The
//! iOS UI test helper, which can't link the Rust library, is checked the same way by the iOS unit
//! tests, and the identifier table in `docs/cloud_backup.md` is checked here

use strum::IntoEnumIterator as _;

use super::types::{CloudBackupRowControl, CloudBackupRowTarget, CloudBackupScreenAction};
use crate::manager::cloud_backup_manager::RecordId;

/// Something on the Cloud Backup screens a UI test finds by its accessibility identifier
///
/// Provider names stay in visible text and never in an identifier, so one identifier names the
/// same control on iOS and Android
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CloudBackupUiElement {
    /// The control that sends a global action, wherever it is drawn
    Action {
        action: CloudBackupScreenAction,
    },
    /// The reason under a disabled action's control
    ActionUnavailable {
        action: CloudBackupScreenAction,
    },
    /// The confirm button of the dialog an action opens before it dispatches
    ActionConfirm {
        action: CloudBackupScreenAction,
    },
    /// A row control
    RowAction {
        target: CloudBackupRowTarget,
    },
    /// The confirm button of the dialog a row control opens before it dispatches
    RowConfirm {
        target: CloudBackupRowTarget,
    },
    /// A wallet's row in any list; on Android a listed row opens its action sheet
    WalletRow {
        record_id: RecordId,
    },
    /// The reason every row in a section shares, drawn once above them
    RowsUnavailable,
    LastOutcome,
    LastOutcomeDismiss,
    HealNotice,
    /// The heal notice's own Turn Off
    HealNoticeTurnOff,
    TurnOffPreparing,
    TurnOffOffline,
    /// Try Again in the offline Turn Off dialog
    TurnOffOfflineTryAgain,
    /// The first Turn Off dialog, which describes what it deletes
    TurnOffReview,
    /// The review's Continue to the final dialog of a two-step Turn Off
    TurnOffContinue,
    /// The review's Check Again, when Cove couldn't check everything
    TurnOffCheckAgain,
    /// The final dialog of a two-step Turn Off
    TurnOffFinal,
    TurnOffDraining,
    FinishingDisable,
    LoadFailed,
    Corrupted,
    /// Android only
    DriveAccountSwitch,
    InventoryChecking,
    InventoryIncomplete,
    /// The support code an incomplete setup shows, to quote when asking for help
    RecoverySupportCode,
    /// Android only: the top-bar menu
    Menu,
    /// The Enable prompt for a backup that already exists
    PromptExistingBackupFound,
    /// The prompt that asks which passkey to use
    PromptPasskeyChoice,
    /// The repair prompt's Create New Passkey, which sends the same action as its Use Existing
    /// Passkey and so needs its own identifier
    PromptPasskeyChoiceCreateNewPasskey,
    PromptMissingPasskeyReminder,
    PromptVerification,
}

#[uniffi::export]
impl CloudBackupUiElement {
    /// The identifier both apps give this element
    pub fn accessibility_identifier(&self) -> String {
        let mut identifier = self.template().to_owned();
        if let Some(action) = self.action() {
            identifier = identifier.replace("<action>", &action.identifier_name());
        }

        if let Some(record_id) = self.record_id() {
            identifier = identifier.replace("<recordId>", &String::from(record_id.clone()));
        }

        if let Some(control) = self.row_control() {
            identifier = identifier.replace("<rowControl>", &control.identifier_name());
        }

        identifier
    }
}

#[uniffi::export]
impl CloudBackupScreenAction {
    /// The action's part of its accessibility identifier: its name in lower camel case
    pub fn identifier_name(&self) -> String {
        <&'static str>::from(self).to_owned()
    }
}

#[uniffi::export]
impl CloudBackupRowControl {
    /// The row control's part of its accessibility identifier: its name in lower camel case
    pub fn identifier_name(&self) -> String {
        <&'static str>::from(self).to_owned()
    }
}

/// Every element both apps tag, with `record-1` standing for a row's record
///
/// The platform contract tests check their own mapping against each of these
#[uniffi::export]
pub fn cloud_backup_ui_elements() -> Vec<CloudBackupUiElement> {
    every_element(&RecordId::from("record-1".to_owned()))
}

impl CloudBackupUiElement {
    /// The identifier, with `<action>`, `<recordId>` and `<rowControl>` standing for the parts an
    /// element names; the docs table lists these forms
    fn template(&self) -> &'static str {
        match self {
            Self::Action { .. } => "cloudBackup.action.<action>",
            Self::ActionUnavailable { .. } => "cloudBackup.action.<action>.unavailable",
            Self::ActionConfirm { .. } => "cloudBackup.action.<action>.confirm",
            Self::RowAction { .. } => "cloudBackup.row.<recordId>.<rowControl>",
            Self::RowConfirm { .. } => "cloudBackup.row.<recordId>.<rowControl>.confirm",
            Self::WalletRow { .. } => "cloudBackup.wallet.<recordId>",
            Self::RowsUnavailable => "cloudBackup.rows.unavailable",
            Self::LastOutcome => "cloudBackup.lastOutcome",
            Self::LastOutcomeDismiss => "cloudBackup.lastOutcome.dismiss",
            Self::HealNotice => "cloudBackup.healNotice",
            Self::HealNoticeTurnOff => "cloudBackup.healNotice.turnOff",
            Self::TurnOffPreparing => "cloudBackup.turnOff.preparing",
            Self::TurnOffOffline => "cloudBackup.turnOff.offline",
            Self::TurnOffOfflineTryAgain => "cloudBackup.turnOff.offline.tryAgain",
            Self::TurnOffReview => "cloudBackup.turnOff.review",
            Self::TurnOffContinue => "cloudBackup.turnOff.continue",
            Self::TurnOffCheckAgain => "cloudBackup.turnOff.checkAgain",
            Self::TurnOffFinal => "cloudBackup.turnOff.final",
            Self::TurnOffDraining => "cloudBackup.turnOff.draining",
            Self::FinishingDisable => "cloudBackup.finishingDisable",
            Self::LoadFailed => "cloudBackup.loadFailed",
            Self::Corrupted => "cloudBackup.corrupted",
            Self::DriveAccountSwitch => "cloudBackup.driveAccountSwitch",
            Self::InventoryChecking => "cloudBackup.inventory.checking",
            Self::InventoryIncomplete => "cloudBackup.inventory.incomplete",
            Self::RecoverySupportCode => "cloudBackup.recovery.supportCode",
            Self::Menu => "cloudBackup.menu",
            Self::PromptExistingBackupFound => "cloudBackup.prompt.existingBackupFound",
            Self::PromptPasskeyChoice => "cloudBackup.prompt.passkeyChoice",
            Self::PromptPasskeyChoiceCreateNewPasskey => {
                "cloudBackup.prompt.passkeyChoice.createNewPasskey"
            }
            Self::PromptMissingPasskeyReminder => "cloudBackup.prompt.missingPasskeyReminder",
            Self::PromptVerification => "cloudBackup.prompt.verification",
        }
    }

    fn action(&self) -> Option<CloudBackupScreenAction> {
        match self {
            Self::Action { action }
            | Self::ActionUnavailable { action }
            | Self::ActionConfirm { action } => Some(*action),
            _ => None,
        }
    }

    fn record_id(&self) -> Option<&RecordId> {
        match self {
            Self::RowAction { target } | Self::RowConfirm { target } => Some(&target.record_id),
            Self::WalletRow { record_id } => Some(record_id),
            _ => None,
        }
    }

    fn row_control(&self) -> Option<CloudBackupRowControl> {
        match self {
            Self::RowAction { target } | Self::RowConfirm { target } => Some(target.control),
            _ => None,
        }
    }
}

/// Every element, each action and row control with every element that names one
fn every_element(record_id: &RecordId) -> Vec<CloudBackupUiElement> {
    use CloudBackupUiElement as E;

    let actions = CloudBackupScreenAction::iter().flat_map(|action| {
        [E::Action { action }, E::ActionUnavailable { action }, E::ActionConfirm { action }]
    });

    let rows = CloudBackupRowControl::iter().flat_map(|control| {
        let target = CloudBackupRowTarget { record_id: record_id.clone(), control };
        [E::RowAction { target: target.clone() }, E::RowConfirm { target }]
    });

    let surfaces = [
        E::WalletRow { record_id: record_id.clone() },
        E::RowsUnavailable,
        E::LastOutcome,
        E::LastOutcomeDismiss,
        E::HealNotice,
        E::HealNoticeTurnOff,
        E::TurnOffPreparing,
        E::TurnOffOffline,
        E::TurnOffOfflineTryAgain,
        E::TurnOffReview,
        E::TurnOffContinue,
        E::TurnOffCheckAgain,
        E::TurnOffFinal,
        E::TurnOffDraining,
        E::FinishingDisable,
        E::LoadFailed,
        E::Corrupted,
        E::DriveAccountSwitch,
        E::InventoryChecking,
        E::InventoryIncomplete,
        E::RecoverySupportCode,
        E::Menu,
        E::PromptExistingBackupFound,
        E::PromptPasskeyChoice,
        E::PromptPasskeyChoiceCreateNewPasskey,
        E::PromptMissingPasskeyReminder,
        E::PromptVerification,
    ];

    actions.chain(rows).chain(surfaces).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use super::{CloudBackupUiElement, every_element};
    use crate::manager::cloud_backup_manager::RecordId;

    const DOCS: &str = include_str!("../../../../../docs/cloud_backup.md");
    const TABLE_HEADING: &str = "### Accessibility Identifiers";

    fn documented_identifiers() -> BTreeSet<String> {
        let section = DOCS.split(TABLE_HEADING).nth(1).expect("the docs have the identifier table");
        let table = section.split("\n#").next().unwrap_or(section);

        table
            .lines()
            .filter(|line| line.starts_with('|'))
            .flat_map(|line| line.split('`').skip(1).step_by(2))
            .filter(|code| code.starts_with("cloudBackup."))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn every_element_has_its_own_identifier() {
        let elements = every_element(&RecordId::from("record-1".to_owned()));
        let mut seen = HashSet::new();
        let duplicates: Vec<_> = elements
            .iter()
            .map(CloudBackupUiElement::accessibility_identifier)
            .filter(|identifier| !seen.insert(identifier.clone()))
            .collect();

        assert_eq!(duplicates, Vec::<String>::new());
        assert_eq!(
            CloudBackupUiElement::Action {
                action: super::CloudBackupScreenAction::AcceptEnablePromptUseExisting
            }
            .accessibility_identifier(),
            "cloudBackup.action.acceptEnablePromptUseExisting"
        );
        assert_eq!(
            CloudBackupUiElement::RowConfirm {
                target: super::CloudBackupRowTarget {
                    record_id: RecordId::from("abc".to_owned()),
                    control: super::CloudBackupRowControl::BackUpFirst,
                },
            }
            .accessibility_identifier(),
            "cloudBackup.row.abc.backUpFirst.confirm"
        );
    }

    /// The docs table lists every identifier form, and nothing the apps no longer use
    #[test]
    fn the_docs_table_lists_exactly_the_identifiers() {
        let elements = every_element(&RecordId::from("record-1".to_owned()));
        let templates: BTreeSet<String> =
            elements.iter().map(|element| element.template().to_owned()).collect();

        assert_eq!(documented_identifiers(), templates);
    }
}
