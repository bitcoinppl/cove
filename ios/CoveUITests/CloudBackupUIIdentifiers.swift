import XCTest

/// The accessibility identifiers UI tests find Cloud Backup's controls and surfaces by
///
/// Rust owns the mapping (`CloudBackupUiElement.accessibilityIdentifier()`, listed in
/// `docs/cloud_backup.md`) and the app reads it from there. UI tests can't link the Rust library,
/// so they use this copy, which `CloudBackupIOSSafetyHelpersTests` checks against every element
/// Rust lists. An action or row control is named by its case name in lower camel case, such as
/// `turnOff` or `backUpFirst`
enum CloudBackupUIIdentifier {
    /// The control that sends a global action, wherever it is drawn
    static func action(_ action: String) -> String {
        "cloudBackup.action.\(action)"
    }

    /// The reason under a disabled action's control
    static func unavailable(_ action: String) -> String {
        "cloudBackup.action.\(action).unavailable"
    }

    /// The confirm button of the dialog an action opens before it dispatches
    static func confirm(_ action: String) -> String {
        "cloudBackup.action.\(action).confirm"
    }

    /// A row control
    static func row(_ recordId: String, _ control: String) -> String {
        "cloudBackup.row.\(recordId).\(control)"
    }

    /// The confirm button of the dialog a row control opens before it dispatches
    static func rowConfirm(_ recordId: String, _ control: String) -> String {
        "cloudBackup.row.\(recordId).\(control).confirm"
    }

    /// A wallet's row in any list
    static func wallet(_ recordId: String) -> String {
        "cloudBackup.wallet.\(recordId)"
    }

    static let rowsUnavailable = "cloudBackup.rows.unavailable"
    static let lastOutcome = "cloudBackup.lastOutcome"
    static let lastOutcomeDismiss = "cloudBackup.lastOutcome.dismiss"
    static let healNotice = "cloudBackup.healNotice"
    static let healNoticeTurnOff = "cloudBackup.healNotice.turnOff"
    static let turnOffPreparing = "cloudBackup.turnOff.preparing"
    static let turnOffOffline = "cloudBackup.turnOff.offline"
    static let turnOffOfflineTryAgain = "cloudBackup.turnOff.offline.tryAgain"
    static let turnOffReview = "cloudBackup.turnOff.review"
    static let turnOffContinue = "cloudBackup.turnOff.continue"
    static let turnOffCheckAgain = "cloudBackup.turnOff.checkAgain"
    static let turnOffFinal = "cloudBackup.turnOff.final"
    static let turnOffDraining = "cloudBackup.turnOff.draining"
    static let finishingDisable = "cloudBackup.finishingDisable"
    static let loadFailed = "cloudBackup.loadFailed"
    static let corrupted = "cloudBackup.corrupted"
    static let driveAccountSwitch = "cloudBackup.driveAccountSwitch"
    static let inventoryChecking = "cloudBackup.inventory.checking"
    static let inventoryIncomplete = "cloudBackup.inventory.incomplete"
    static let recoverySupportCode = "cloudBackup.recovery.supportCode"
    static let menu = "cloudBackup.menu"
    static let promptExistingBackupFound = "cloudBackup.prompt.existingBackupFound"
    static let promptPasskeyChoice = "cloudBackup.prompt.passkeyChoice"
    static let promptPasskeyChoiceCreateNewPasskey = "cloudBackup.prompt.passkeyChoice.createNewPasskey"
    static let promptMissingPasskeyReminder = "cloudBackup.prompt.missingPasskeyReminder"
    static let promptVerification = "cloudBackup.prompt.verification"
}

extension XCUIElementTypeQueryProvider {
    /// An alert's button found by its accessibility identifier
    ///
    /// On iOS 27 XCUITest lists an alert action that carries an identifier twice, as the action and as
    /// UIKit's action view nested inside it, with the same identifier and label. VoiceOver still reads
    /// the action once, so this takes the first match instead of requiring a single one
    func alertButton(_ identifier: String) -> XCUIElement {
        buttons.matching(identifier: identifier).firstMatch
    }
}
