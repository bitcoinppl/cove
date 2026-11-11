import SwiftUI

enum CloudBackupDestructiveConfirmation: Equatable {
    case recreate
    case reinitialize
}

/// Which Turn Off dialog is up
///
/// Only the step is kept here; the text and buttons are read from the review Rust holds when the
/// dialog draws, so a dialog never shows an impact Rust has replaced
enum CloudBackupTurnOffDialog: Equatable {
    /// What Turn Off deletes, with what to do instead
    case review(reviewId: UInt64)

    /// The final confirmation a two-step review asks for
    case confirm(reviewId: UInt64)

    /// Turn Off needs a connection
    case offline
}

enum CloudBackupDetailDialog {
    case destructive(CloudBackupDestructiveConfirmation)
    case recoverOtherBackups
}

enum CloudBackupDetailAlert {
    case rowDelete(CloudBackupListedRow, CloudBackupDeleteConfirmation)
    case undecryptableWalletDeletion(UInt32)
    case turnOff(CloudBackupTurnOffDialog)
    case otherBackupsRecoveryResult(OtherBackupsRecoveryResult)
    case otherBackupsDeleteConfirmation
    case otherBackupsFinalDeleteConfirmation
}

enum CloudBackupDetailPresentation {
    case dialog(CloudBackupDetailDialog)
    case alert(CloudBackupDetailAlert)

    var turnOffDialog: CloudBackupTurnOffDialog? {
        guard case let .alert(.turnOff(dialog)) = self else { return nil }
        return dialog
    }
}

/// The Turn Off dialog Rust's disable state calls for
enum CloudBackupTurnOffDialogDemand: Equatable {
    /// The review Rust holds, which starts from its first dialog
    case review(reviewId: UInt64)

    /// Turn Off needs a connection
    case offline

    /// No Turn Off dialog, as while Turn Off prepares, drains or isn't running
    case none

    init(_ disable: CloudBackupDisableScreen) {
        switch disable {
        case let .reviewing(review): self = .review(reviewId: review.reviewId)
        case .offline: self = .offline
        case .idle, .preparing, .draining: self = .none
        }
    }
}

extension PresentationTransitionCoordinator where Presentation == CloudBackupDetailPresentation {
    /// Shows the Turn Off dialog `demand` calls for, and closes one it no longer calls for
    ///
    /// Reads the pending presentation, not only the one on screen: a review's Continue queues its
    /// confirmation while the review alert dismisses, and a sync in that wait, such as the one the
    /// scene's return to active runs, must keep that confirmation instead of bringing the review
    /// back. A dialog Rust no longer calls for is dropped whether it is up or still queued
    func syncTurnOffDialog(_ demand: CloudBackupTurnOffDialogDemand) {
        let pending = pendingPresentation?.turnOffDialog

        switch demand {
        case let .review(reviewId):
            // a changed impact gets a new review id, so the review starts again from its first dialog
            switch pending {
            case .review(reviewId), .confirm(reviewId):
                return
            default:
                present(.alert(.turnOff(.review(reviewId: reviewId))))
            }

        case .offline:
            guard pending != .offline else { return }
            present(.alert(.turnOff(.offline)))

        case .none:
            discard { $0.turnOffDialog != nil }
        }
    }
}

extension View {
    func cloudBackupDetailPresentations(
        screen: CloudBackupScreen,
        manager: CloudBackupManager,
        coordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>
    ) -> some View {
        modifier(CloudBackupDetailPresentationModifier(
            screen: screen,
            manager: manager,
            coordinator: coordinator
        ))
    }
}

private struct CloudBackupDetailPresentationModifier: ViewModifier {
    let screen: CloudBackupScreen
    let manager: CloudBackupManager
    let coordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var actions: CloudBackupActions {
        screen.actions
    }

    private var dialog: CloudBackupDetailDialog? {
        guard case let .dialog(dialog) = coordinator.currentPresentation?.item else {
            return nil
        }

        return dialog
    }

    private var alert: CloudBackupDetailAlert? {
        guard case let .alert(alert) = coordinator.currentPresentation?.item else {
            return nil
        }

        return alert
    }

    private var review: CloudBackupDisableReview? {
        guard case let .reviewing(review) = screen.configured?.disable else { return nil }
        return review
    }

    private var dialogIsPresented: Binding<Bool> {
        coordinator.isPresented { presentation in
            if case .dialog = presentation { true } else { false }
        }
    }

    private var alertIsPresented: Binding<Bool> {
        coordinator.isPresented { presentation in
            if case .alert = presentation { true } else { false }
        }
    }

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                dialog.map(dialogTitle) ?? "Cloud Backup",
                isPresented: dialogIsPresented,
                titleVisibility: .visible,
                presenting: dialog,
                actions: dialogActions,
                message: dialogMessage
            )
            .alert(
                alert.map(alertTitle) ?? "Cloud Backup",
                isPresented: alertIsPresented,
                presenting: alert,
                actions: alertActions,
                message: alertMessage
            )
    }

    private func dialogTitle(_ dialog: CloudBackupDetailDialog) -> String {
        switch dialog {
        case let .destructive(confirmation):
            switch confirmation {
            case .recreate: "Recreate Backup Index"
            case .reinitialize: "Reinitialize Cloud Backup"
            }
        case .recoverOtherBackups:
            "Recover wallets from another passkey?"
        }
    }

    @ViewBuilder
    private func dialogActions(_ dialog: CloudBackupDetailDialog) -> some View {
        switch dialog {
        case let .destructive(confirmation):
            Button(destructiveActionTitle(confirmation), role: .destructive) {
                performDestructiveAction(confirmation)
            }
            .disabled(!actions.isAvailable(action: confirmation.action))

            Button("Cancel", role: .cancel) {}

        case .recoverOtherBackups:
            Button("Try Passkey", action: recoverOtherBackups)
                .disabled(!actions.isAvailable(action: .recoverOtherBackups))

            Button("Cancel", role: .cancel) {}
        }
    }

    @ViewBuilder
    private func dialogMessage(_ dialog: CloudBackupDetailDialog) -> some View {
        switch dialog {
        case let .destructive(confirmation):
            switch confirmation {
            case .recreate:
                Text(
                    "This will rebuild the backup index from wallets on this device. Wallets that only exist in the cloud backup will no longer be referenced."
                )
            case .reinitialize:
                Text(
                    "This will replace your entire cloud backup. Wallets that only exist in the current cloud backup will be lost."
                )
            }
        case .recoverOtherBackups:
            Text(
                "This will use the selected passkey once to decrypt these other backups. Your current Cloud Backup passkey will not change."
            )
        }
    }

    private func alertTitle(_ alert: CloudBackupDetailAlert) -> String {
        switch alert {
        case let .rowDelete(row, _):
            "Delete \(row.record.wallet?.name ?? "This Backup")?"
        case let .undecryptableWalletDeletion(count):
            "Delete \(count) Inaccessible Wallet \(count == 1 ? "Backup" : "Backups")?"
        case let .turnOff(dialog):
            CloudBackupTurnOffCopy.title(dialog)
        case .otherBackupsRecoveryResult:
            "Wallets Recovered"
        case .otherBackupsDeleteConfirmation:
            "Delete Other Cloud Backups?"
        case .otherBackupsFinalDeleteConfirmation:
            "This Cannot Be Undone"
        }
    }

    @ViewBuilder
    private func alertActions(_ alert: CloudBackupDetailAlert) -> some View {
        switch alert {
        case let .rowDelete(row, _):
            Button("Delete Forever", role: .destructive) {
                deleteRow(row)
            }
            .disabled(!(row.actions.delete?.availability.isAvailable ?? false))

            Button("Cancel", role: .cancel) {}

        case .undecryptableWalletDeletion:
            Button(
                "Delete Backups",
                role: .destructive,
                action: deleteUndecryptableWalletBackups
            )
            .disabled(!actions.isAvailable(action: .deleteUnreadableBackups))

            Button("Cancel", role: .cancel) {}

        case let .turnOff(dialog):
            CloudBackupTurnOffDialogActions(
                dialog: dialog,
                review: review,
                actions: actions,
                manager: manager,
                coordinator: coordinator
            )

        case .otherBackupsRecoveryResult:
            Button("Verify Current Passkey", action: verifyCurrentPasskey)
                .disabled(!actions.isAvailable(action: .verify))
            Button("Done", role: .cancel) {}

        case .otherBackupsDeleteConfirmation:
            Button("Continue", role: .destructive, action: presentFinalOtherBackupsDeleteConfirmation)
                .disabled(!actions.isAvailable(action: .deleteOtherBackups))

            Button("Cancel", role: .cancel) {}

        case .otherBackupsFinalDeleteConfirmation:
            Button("Delete", role: .destructive, action: deleteOtherBackups)
                .disabled(!actions.isAvailable(action: .deleteOtherBackups))

            Button("Cancel", role: .cancel) {}
        }
    }

    @ViewBuilder
    private func alertMessage(_ alert: CloudBackupDetailAlert) -> some View {
        switch alert {
        case let .rowDelete(_, confirmation):
            switch confirmation {
            case .standard:
                Text("This backup will be permanently removed from iCloud.")
            case .mayBeOnlyCopy:
                Text(
                    "This wallet isn't on this device, so this may be its only backup. Without its recovery words, it can't be recovered."
                )
            }
        case .undecryptableWalletDeletion:
            Text(
                "Cove will check these files again and delete only backups for wallets that are not on this device and still cannot be decrypted. This cannot be undone."
            )
        case let .turnOff(dialog):
            Text(CloudBackupTurnOffCopy.message(dialog, review: review))
        case let .otherBackupsRecoveryResult(result):
            Text(result.message)
        case .otherBackupsDeleteConfirmation:
            Text("This will permanently remove these other backups from iCloud.")
        case .otherBackupsFinalDeleteConfirmation:
            Text(
                "These backups cannot be recovered later, even if you find the passkey that currently protects them."
            )
        }
    }

    private func destructiveActionTitle(
        _ confirmation: CloudBackupDestructiveConfirmation
    ) -> String {
        switch confirmation {
        case .recreate: "Recreate"
        case .reinitialize: "Reinitialize"
        }
    }

    private func performDestructiveAction(_ confirmation: CloudBackupDestructiveConfirmation) {
        coordinator.dismissCurrentPresentation()

        switch confirmation {
        case .recreate:
            manager.dispatch(action: .recreateManifest)
        case .reinitialize:
            manager.dispatch(action: .reinitializeBackup)
        }
    }

    private func recoverOtherBackups() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .recoverOtherBackups)
    }

    private func deleteRow(_ row: CloudBackupListedRow) {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .deleteCloudWallet(row.recordId))
    }

    private func verifyCurrentPasskey() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .startVerification(.cloudBackupDetail))
    }

    private func presentFinalOtherBackupsDeleteConfirmation() {
        coordinator.transition(to: .alert(.otherBackupsFinalDeleteConfirmation))
    }

    private func deleteOtherBackups() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .deleteOtherBackups)
    }

    private func deleteUndecryptableWalletBackups() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .deleteUndecryptableWalletBackups)
    }
}

extension CloudBackupDestructiveConfirmation {
    /// The screen action the confirmation sends
    var action: CloudBackupScreenAction {
        switch self {
        case .recreate: .recreateManifest
        case .reinitialize: .reinitializeBackup
        }
    }
}

/// The buttons of the Turn Off dialogs
///
/// A button that would leave Turn Off cancels the review first, so the user taps Turn Off again
/// afterwards and nothing continues into Turn Off on its own
private struct CloudBackupTurnOffDialogActions: View {
    let dialog: CloudBackupTurnOffDialog
    let review: CloudBackupDisableReview?
    let actions: CloudBackupActions
    let manager: CloudBackupManager
    let coordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        switch dialog {
        case .offline:
            Button("Try Again", action: tryAgain)
                .disabled(!actions.isAvailable(action: .turnOff))
                .accessibilityIdentifier("cloudBackup.turnOff.offline.tryAgain")

            cancelButton

        case let .review(reviewId):
            if let review, review.reviewId == reviewId {
                CloudBackupTurnOffReviewActions(
                    review: review,
                    actions: actions,
                    manager: manager,
                    coordinator: coordinator
                )
            }

            cancelButton

        case let .confirm(reviewId):
            Button("Delete and Turn Off", role: .destructive) {
                confirm(reviewId)
            }
            .disabled(!actions.isAvailable(action: .confirmDisable))
            .accessibilityIdentifier("cloudBackup.turnOff.confirm")

            cancelButton
        }
    }

    /// never disabled: a dialog must always close, and a rejected cancel comes back as the last outcome
    private var cancelButton: some View {
        Button("Cancel", role: .cancel) {
            manager.dispatch(action: .cancelDisable)
        }
        .accessibilityIdentifier("cloudBackup.turnOff.cancel")
    }

    private func tryAgain() {
        manager.dispatch(action: .disableCloudBackup)
    }

    private func confirm(_ reviewId: UInt64) {
        manager.dispatch(action: .confirmDisable(reviewId: reviewId))
    }
}

/// The first dialog's buttons, by the shape of the impact
private struct CloudBackupTurnOffReviewActions: View {
    let review: CloudBackupDisableReview
    let actions: CloudBackupActions
    let manager: CloudBackupManager
    let coordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var impact: CloudBackupDisableImpact {
        review.impact
    }

    var body: some View {
        switch impact.confirmation {
        case .one:
            Button("Turn Off and Delete Backups", role: .destructive) {
                manager.dispatch(action: .confirmDisable(reviewId: review.reviewId))
            }
            .disabled(!actions.isAvailable(action: .confirmDisable))
            .accessibilityIdentifier("cloudBackup.turnOff.confirm")

        case .two:
            CloudBackupTurnOffLeaveActions(impact: impact, actions: actions, manager: manager)

            Button("Continue", role: .destructive) {
                coordinator.transition(to: .alert(.turnOff(.confirm(reviewId: review.reviewId))))
            }
            .disabled(!actions.isAvailable(action: .confirmDisable))
            .accessibilityIdentifier("cloudBackup.turnOff.continue")
        }
    }
}

/// The review buttons that act on what it warned about before anything is deleted
private struct CloudBackupTurnOffLeaveActions: View {
    let impact: CloudBackupDisableImpact
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    /// Rust couldn't finish counting what Turn Off would remove
    private var couldNotCheck: Bool {
        if case .complete = impact.assessment { return false }
        return true
    }

    /// Restore and Verify cancel the review before they start anything else, so they wait on
    /// CancelDisable
    var body: some View {
        if impact.notOnDevice.total > 0 {
            Button("Restore Them First") {
                leaveTurnOff(then: .startRestoreAll)
            }
            .disabled(!actions.isAvailable(action: .cancelDisable))
            .accessibilityIdentifier("cloudBackup.turnOff.restoreFirst")
        }

        // placeholder until Rust exports a VerifyFirst action: then draw Verify First only when
        // Rust offers it, and dispatch that action alone, gated on its own availability
        if impact.unchecked > 0 {
            Button("Verify First") {
                leaveTurnOff(then: .startVerification(.cloudBackupDetail))
            }
            .disabled(!actions.isAvailable(action: .cancelDisable))
            .accessibilityIdentifier("cloudBackup.turnOff.verifyFirst")
        }

        // a second Turn Off re-prepares the review Rust holds, so Check Again stays in Turn Off
        if couldNotCheck {
            Button("Check Again") {
                manager.dispatch(action: .disableCloudBackup)
            }
            .disabled(!actions.isAvailable(action: .turnOff))
            .accessibilityIdentifier("cloudBackup.turnOff.checkAgain")
        }
    }

    private func leaveTurnOff(then action: CloudBackupManagerAction) {
        manager.dispatch(action: .cancelDisable)
        manager.dispatch(action: action)
    }
}

/// The Turn Off dialog text, built from the counts and names Rust supplies
enum CloudBackupTurnOffCopy {
    static let provider = "iCloud"

    static func title(_ dialog: CloudBackupTurnOffDialog) -> String {
        switch dialog {
        case .review: "Turn off Cloud Backup?"
        case .confirm: "Delete these backups and turn off Cloud Backup?"
        case .offline: "Can't Reach \(provider)"
        }
    }

    static func message(_ dialog: CloudBackupTurnOffDialog, review: CloudBackupDisableReview?) -> String {
        switch dialog {
        case .offline:
            "Cove can't reach \(provider) right now. Turning off Cloud Backup needs a connection."
        case .confirm:
            "This can't be undone."
        case .review:
            review.map { reviewMessage($0) } ?? ""
        }
    }

    /// Each line that applies to the impact, in a fixed order
    static func reviewMessage(_ review: CloudBackupDisableReview) -> String {
        let impact = review.impact
        var lines: [String] = []

        if review.changedSinceConfirmation {
            lines.append("Cloud Backup changed since you checked. Review what will be deleted.")
        }

        if case .complete = impact.assessment {} else {
            lines.append(
                "Cove couldn't check what's in Cloud Backup. Wallets that aren't on this device may be deleted."
            )
        }

        if impact.ownBackups > 0 {
            lines.append(
                "This deletes the backups of your \(walletCount(impact.ownBackups)) from \(provider). Your wallets stay on this device and will no longer be backed up."
            )
        }

        if impact.extraCopies > 0 {
            lines.append(
                "This includes \(count(impact.extraCopies, "extra copy", "extra copies")), and any labels saved only in them."
            )
        }

        if impact.notOnDevice.total > 0 {
            lines.append(
                "These wallets are only in Cloud Backup: \(impact.notOnDevice.listText). Turning off Cloud Backup deletes them. Without their recovery words, they can't be recovered."
            )
        }

        if impact.ownSecretMissingLocally.total > 0 {
            lines.append(
                "The recovery words for these wallets aren't on this device: \(impact.ownSecretMissingLocally.listText). Turning off Cloud Backup deletes their backups."
            )
        }

        if impact.unchecked > 0 {
            lines.append(
                "Cove can't check \(count(impact.unchecked, "backup", "backups")) without your passkey. They may be wallets that aren't on this device."
            )
        }

        let unreadable = impact.unreadable + impact.unsupported
        if unreadable > 0 {
            lines.append(
                "Cove can't read \(count(unreadable, "backup", "backups")). They may be damaged, or made by a newer version of Cove. They will be deleted too."
            )
        }

        if impact.otherBackups > 0 {
            lines.append(
                "\(count(impact.otherBackups, "backup", "backups")) made with another passkey won't be deleted. To remove them later, turn Cloud Backup on again, or delete them from \(provider) storage settings."
            )
        }

        return lines.joined(separator: "\n\n")
    }

    private static func walletCount(_ count: UInt32) -> String {
        Self.count(count, "wallet", "wallets")
    }

    private static func count(_ count: UInt32, _ singular: String, _ plural: String) -> String {
        "\(count) \(count == 1 ? singular : plural)"
    }
}
