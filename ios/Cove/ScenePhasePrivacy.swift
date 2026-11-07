import SwiftUI

/// What the root view knows when the scene phase changes, for deciding how to protect the app
struct ScenePhasePrivacyContext: Equatable {
    var oldPhase: ScenePhase
    var newPhase: ScenePhase
    var isAuthEnabled: Bool
    var isUsingBiometrics: Bool
    var isNfcOperationActive: Bool
}

/// How a scene phase change protects the app: the privacy cover, the app lock and presented sheets
///
/// An NFC scan's system sheet makes the app inactive. Covering the app, locking it or dismissing its
/// sheets would interrupt the scan, which looped a TAPSIGNER import with auth enabled, so an active
/// NFC operation leaves all three alone
struct ScenePhasePrivacyOutcome: Equatable {
    enum CoverRecovery: Equatable {
        case unchanged

        /// Stops clearing a cover that a brief trip to inactive left up
        case cancelled

        /// Clears the cover once the app is back to active, replacing any earlier attempt
        case restarted
    }

    var showsCover = false
    var coverRecovery = CoverRecovery.unchanged

    /// Locks for leaving the foreground
    var locks = false

    /// Dismisses presented view controllers and ends editing
    var dismissesSheets = false
}

extension ScenePhasePrivacyOutcome {
    init(_ context: ScenePhasePrivacyContext) {
        let leavesActive = context.oldPhase == .active && context.newPhase != .active
        showsCover = leavesActive && !context.isNfcOperationActive

        switch context.newPhase {
        case .inactive:
            guard context.isAuthEnabled, !context.isUsingBiometrics, context.oldPhase == .active else {
                return
            }

            coverRecovery = .restarted

        case .background:
            // the cached auth type can lag behind or misread the saved one, and Rust skips the
            // lock only when it can read that no authentication is set up
            guard context.isAuthEnabled else {
                locks = true
                return
            }

            coverRecovery = .cancelled
            guard !context.isNfcOperationActive else { return }

            showsCover = true

            // lock even when already locked, so a lock another owner can release still leaves
            // this one needing authentication
            locks = true
            dismissesSheets = true

        default:
            return
        }
    }
}
