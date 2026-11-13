import XCTest

/// Sync Now and Verify Now from Cloud Backup detail, with Fixture Wallet One restored from
/// onboarding
///
/// A wallet added after the restore has its first upload rejected for quota, the one upload error
/// Cove doesn't retry on its own, so the detail lists it as needing a sync until Sync Now uploads it
final class CloudBackupSyncNowFullAppUITests: XCTestCase {
    private static let walletOneRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"

    /// A failure that requests no detail refresh can land after the last one, so the detail is
    /// reopened this many times before the test gives up on seeing it
    private static let failureReopenAttempts = 6

    /// The Settings row's title while a wallet's upload has failed
    private static let errorRowTitle = "Cloud Backup Error"

    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        robot = CloudBackupHostRobot(app: app)
    }

    override func tearDown() {
        CloudBackupHostControl.tearDown(self, app: app)
        super.tearDown()
    }

    func testSyncNowUploadsPendingWalletAndRefreshesDetail() throws {
        let (control, pending) = try restoreFixtureWithPendingWallet()
        control.assertNextActions("a wallet waiting to upload should offer Sync Now", available: ["syncUnsynced", "turnOff"])

        tapSyncNow()

        control.waitUntil("Sync Now should upload the pending wallet") { acceptedNewWalletUploads($0) == 1 }
        assertSyncedDetailShows(pending, control)
    }

    func testOfflineSyncNowUploadsNothingThenUploadsAfterReconnect() throws {
        let (control, pending) = try restoreFixtureWithPendingWallet()
        control.assertNextActions("a wallet waiting to upload should offer Sync Now", available: ["syncUnsynced", "turnOff"])

        try control.setNetworkConnected(false)
        let beforeOfflineSync = control.state
        tapSyncNow()

        control.waitForAppState("Sync Now offline should fail with the offline error") {
            $0.sync == "failed" && $0.syncError?.contains("Reconnect to the internet") == true
        }
        robot.scrollToHittable(robot.staticText(containing: "Reconnect to the internet"))

        // the host counts every upload call, even one it refuses offline, while iCloud would
        // accept a write offline and stage it, so offline Sync Now must not try one at all
        let afterOfflineSync = control.state
        XCTAssertEqual(
            afterOfflineSync.count(.walletUpload),
            beforeOfflineSync.count(.walletUpload),
            "Sync Now tried a wallet upload offline"
        )
        XCTAssertEqual(
            afterOfflineSync.count(.masterUpload),
            beforeOfflineSync.count(.masterUpload),
            "Sync Now tried a master key upload offline"
        )
        XCTAssertEqual(acceptedNewWalletUploads(afterOfflineSync), 0, "nothing may be uploaded while offline")

        try control.setNetworkConnected(true)
        tapSyncNow()

        control.waitUntil("Sync Now after reconnecting should upload the pending wallet") {
            acceptedNewWalletUploads($0) == 1
        }
        assertSyncedDetailShows(pending, control)
    }

    func testCancelledVerificationRetriesWithTheSamePasskey() throws {
        let control = try restoreFixtureAndOpenDetail(verify: true)
        control.assertNextActions("a verified backup should offer Verify", available: ["verify", "turnOff"])
        let before = control.state
        try control.answerNextPasskeyAssertions([.userCancelled, .success])

        let verifyAgain = app.buttons["Verify Again"]
        robot.scrollToHittable(verifyAgain)
        verifyAgain.tap()

        control.waitForAppState("cancelling the passkey should leave verification not completed") {
            $0.verification == "notCompleted"
        }
        robot.scrollToHittable(app.staticTexts["Cloud Backup Not Verified"])
        XCTAssertEqual(passkeyAssertions(control.state), passkeyAssertions(before) + 1)

        let verifyNow = app.buttons["Verify Now"]
        robot.scrollToHittable(verifyNow)
        verifyNow.tap()

        control.waitForAppState("verifying again should verify the backup with the same passkey") {
            $0.verification == "verified"
        }
        robot.scrollToHittable(app.staticTexts["Backup verified"])

        let after = control.state
        XCTAssertEqual(passkeyAssertions(after), passkeyAssertions(before) + 2)
        XCTAssertEqual(after.count(.passkeyCreate), before.count(.passkeyCreate), "retrying verification created a passkey")

        // the provider holds one passkey, the one the restore discovered, so any other credential
        // would be a passkey a real phone might not have
        let knownCredential = Data(CloudBackupUITestControl.passkeyCredentialId.utf8).map { String(format: "%02x", $0) }.joined()
        let retryCredentials = after.authenticatedCredentialIds.dropFirst(before.authenticatedCredentialIds.count)
        XCTAssertFalse(retryCredentials.isEmpty, "verification authenticated with no known credential")
        XCTAssertTrue(
            retryCredentials.allSatisfy { $0 == knownCredential },
            "verification must authenticate with the restored passkey, got \(Array(retryCredentials))"
        )
    }

    // MARK: - Setup

    /// Restores Fixture Wallet One and opens Cloud Backup detail once the entry refresh has listed
    /// the cloud, with the backup not verified, then, with `verify`, verifies it with Verify Now
    private func restoreFixtureAndOpenDetail(verify: Bool) throws -> CloudBackupHostControl {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [Self.walletOneRecordId])
        )
        robot.launch(control: control)
        robot.restoreFromCloudBackup()

        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the restored backup never listed only wallet one", timeout: 60) {
            $0.inventoryLoaded && !$0.awaitingUploadConfirmation
                && $0.wallets.map(\.recordId) == [Self.walletOneRecordId]
        }

        if verify {
            robot.verifyRestoredBackup(control)
        } else {
            robot.assertRestoredBackupNotVerified(control)
        }

        return control
    }

    /// Adds the test mnemonic as a wallet whose automatic upload the cloud rejects, so the detail
    /// offers Sync Now for it, and returns its row
    private func restoreFixtureWithPendingWallet() throws -> (CloudBackupHostControl, CloudBackupUITestAppState.Wallet) {
        let control = try restoreFixtureAndOpenDetail(verify: false)
        try control.update { $0.newWalletUploadFailures = [.quotaExceeded] }

        robot.goBack()
        let backToWallet = app.navigationBars["Settings"].buttons.firstMatch
        XCTAssertTrue(backToWallet.waitForExistence(timeout: 10))
        backToWallet.tap()
        app.importTestMnemonicFromAddWallet()

        control.waitUntil("the automatic upload of the new wallet should be rejected") {
            $0.uploads.filter { $0.isNewWallet && !$0.accepted }.count == 1
        }

        // the new wallet changed the backup, so the root asks whether to verify it, over the
        // sidebar Settings is opened from
        let verifyPrompt = app.descendants(matching: .any)[CloudBackupUIIdentifier.promptVerification]
        XCTAssertTrue(verifyPrompt.waitForExistence(timeout: 15), "a new wallet should ask to verify the backup")
        verifyPrompt.buttons["Not Now"].tap()
        XCTAssertTrue(verifyPrompt.waitForNonExistence(timeout: 10), "Not Now should close the verify prompt")

        // Sync Now skips a wallet still uploading, so wait for the detail to show the rejected
        // upload's saved failure; the failure itself requests no refresh, so a refresh that ran
        // before it keeps showing the dirty wallet until the detail is opened again
        let showsFailure: (CloudBackupUITestAppState) -> Bool = { state in
            state.wallets.contains { $0.recordId != Self.walletOneRecordId && $0.needsSync && $0.status == "failed" }
        }

        // Settings names the rejection before the detail is opened
        robot.openSettings()
        XCTAssertTrue(
            robot.staticText(containing: "Your cloud storage is full").waitForExistence(timeout: 15),
            "Settings should show the rejected upload's quota error"
        )
        robot.openCloudBackupDetail(rowTitle: Self.errorRowTitle)
        for _ in 0 ..< Self.failureReopenAttempts {
            if waitForAppState(control, timeout: 5, showsFailure) { break }

            robot.goBack()
            robot.openCloudBackupDetail(rowTitle: Self.errorRowTitle)
        }
        control.waitForAppState("the detail should list the new wallet as needing sync after its upload failed", timeout: 1, showsFailure)

        let pending = try XCTUnwrap(control.appState?.wallets.first { $0.recordId != Self.walletOneRecordId })
        robot.scrollToHittable(app.buttons["Sync Now"])
        XCTAssertEqual(acceptedNewWalletUploads(control.state), 0)

        return (control, pending)
    }

    // MARK: - Steps and assertions

    private func tapSyncNow() {
        let syncNow = app.buttons["Sync Now"]
        robot.scrollToHittable(syncNow)
        syncNow.tap()
    }

    /// The sync finished, the detail refreshed to list the wallet as confirmed, and the screen
    /// shows that too: the wallet's row says Confirmed and Sync Now is gone
    private func assertSyncedDetailShows(_ wallet: CloudBackupUITestAppState.Wallet, _ control: CloudBackupHostControl) {
        control.waitForAppState("the sync should finish and the detail refresh to show the wallet up to date") {
            $0.sync == "idle" && $0.inventoryLoaded && $0.wallets.allSatisfy { !$0.needsSync }
                && $0.wallets.contains { $0.recordId == wallet.recordId && $0.status == "confirmed" }
        }

        let row = app.descendants(matching: .any)[CloudBackupUIIdentifier.wallet(wallet.recordId)]
        robot.scrollToHittable(row)
        XCTAssertTrue(row.label.contains(wallet.name), "the row is not the synced wallet: \(row.label)")
        XCTAssertTrue(row.label.contains("Confirmed"), "the synced wallet's row is not Confirmed: \(row.label)")
        XCTAssertFalse(row.label.contains("Failed"), "the synced wallet's row still shows its failure: \(row.label)")
        XCTAssertTrue(
            app.buttons["Sync Now"].waitForNonExistence(timeout: 10),
            "Sync Now should disappear once nothing needs sync"
        )
    }

    /// Waits without failing, for a condition the caller retries
    private func waitForAppState(
        _ control: CloudBackupHostControl,
        timeout: TimeInterval,
        _ condition: (CloudBackupUITestAppState) -> Bool
    ) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let state = control.appState, condition(state) { return true }

            Thread.sleep(forTimeInterval: 0.1)
        }

        return false
    }

    private func acceptedNewWalletUploads(_ state: CloudBackupUITestHostState) -> Int {
        state.uploads.filter { $0.isNewWallet && $0.accepted }.count
    }

    private func passkeyAssertions(_ state: CloudBackupUITestHostState) -> Int {
        state.count(.passkeyAuthenticate) + state.count(.passkeyDiscover)
    }
}
