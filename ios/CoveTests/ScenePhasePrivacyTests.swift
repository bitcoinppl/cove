@testable import Cove
import CoveCore
import SwiftUI
import XCTest

/// How leaving the foreground covers, locks and dismisses, and how an NFC scan, whose system sheet
/// makes the app inactive, holds all three off
@MainActor
final class ScenePhasePrivacyTests: XCTestCase {
    private static let leavingActive: [ScenePhase] = [.inactive, .background]

    func testNfcScanKeepsTheAppUncoveredUnlockedAndItsSheetsUp() {
        for context in contexts(nfc: true) {
            let outcome = ScenePhasePrivacyOutcome(context)

            XCTAssertFalse(outcome.showsCover, "\(context)")
            XCTAssertFalse(outcome.dismissesSheets, "\(context)")

            // with auth off the lock is Rust's to skip, since the cached auth type can be stale
            XCTAssertEqual(outcome.locks, !context.isAuthEnabled && context.newPhase == .background, "\(context)")
        }
    }

    func testLeavingTheForegroundWithoutNfcCoversLocksAndDismisses() {
        for context in contexts(nfc: false) {
            let outcome = ScenePhasePrivacyOutcome(context)
            let backgrounds = context.newPhase == .background

            XCTAssertEqual(outcome.showsCover, context.oldPhase == .active || context.isAuthEnabled && backgrounds, "\(context)")
            XCTAssertEqual(outcome.locks, backgrounds, "\(context)")
            XCTAssertEqual(outcome.dismissesSheets, backgrounds && context.isAuthEnabled, "\(context)")
        }
    }

    /// A brief trip to inactive with a PIN clears its cover once the app is back, and going on to
    /// the background stops that, NFC scan or not
    func testCoverRecoveryFollowsAPinLockThroughInactive() {
        for context in contexts(nfc: true) + contexts(nfc: false) {
            let expected: ScenePhasePrivacyOutcome.CoverRecovery =
                switch (context.newPhase, context.isAuthEnabled) {
                case (.inactive, true) where !context.isUsingBiometrics: .restarted
                case (.background, true): .cancelled
                default: .unchanged
                }

            XCTAssertEqual(ScenePhasePrivacyOutcome(context).coverRecovery, expected, "\(context)")
        }
    }

    func testReturningToTheForegroundLeavesTheCoverAndLockToTheActivePhase() {
        for oldPhase in Self.leavingActive {
            for flags in authAndNfcFlags() {
                let context = ScenePhasePrivacyContext(
                    oldPhase: oldPhase,
                    newPhase: .active,
                    isAuthEnabled: flags.auth,
                    isUsingBiometrics: flags.biometrics,
                    isNfcOperationActive: flags.nfc
                )

                XCTAssertEqual(ScenePhasePrivacyOutcome(context), ScenePhasePrivacyOutcome(), "\(context)")
            }
        }
    }

    /// Each NFC owner the app holds counts as an active operation on its own
    func testEveryAppNfcOwnerCountsAsAnActiveOperation() async throws {
        try await bootstrapIfNeeded()
        let app = AppManager.shared

        XCTAssertNil(app.tapSignerNfc, "the test host should hold no TAPSIGNER session")
        XCTAssertFalse(app.isNfcOperationActive, "the test host should hold no NFC scan")

        app.nfcReader.isScanning = true
        XCTAssertTrue(app.isNfcOperationActive, "a tag read is an NFC operation")
        app.nfcReader.isScanning = false

        app.nfcWriter.isScanning = true
        XCTAssertTrue(app.isNfcOperationActive, "a tag write is an NFC operation")
        app.nfcWriter.isScanning = false

        XCTAssertFalse(app.isNfcOperationActive)
    }

    // MARK: - Helpers

    /// Every way out of the foreground: straight from active, and on from inactive to background,
    /// with auth and biometrics on and off
    private func contexts(nfc: Bool) -> [ScenePhasePrivacyContext] {
        let transitions: [(ScenePhase, ScenePhase)] = [
            (.active, .inactive),
            (.active, .background),
            (.inactive, .background),
        ]

        return transitions.flatMap { oldPhase, newPhase in
            authAndNfcFlags().map { flags in
                ScenePhasePrivacyContext(
                    oldPhase: oldPhase,
                    newPhase: newPhase,
                    isAuthEnabled: flags.auth,
                    isUsingBiometrics: flags.biometrics,
                    isNfcOperationActive: nfc
                )
            }
        }
    }

    private func authAndNfcFlags() -> [(auth: Bool, biometrics: Bool, nfc: Bool)] {
        [false, true].flatMap { auth in
            [false, true].flatMap { biometrics in
                [false, true].map { nfc in (auth, biometrics, nfc) }
            }
        }
    }

    private func bootstrapIfNeeded() async throws {
        do {
            _ = try await bootstrap()
        } catch AppInitError.AlreadyCalled(_) {}

        // the test host app bootstraps on its own, so AlreadyCalled can return before it finishes
        let deadline = Date().addingTimeInterval(30)
        while bootstrapProgress() != .complete {
            try XCTUnwrap(Date() < deadline ? true : nil, "bootstrap did not complete")
            try await Task.sleep(for: .milliseconds(50))
        }
    }
}
