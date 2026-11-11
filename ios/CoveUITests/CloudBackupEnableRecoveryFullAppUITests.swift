import XCTest

/// Fresh Cloud Backup enables against a scripted cloud that accepts writes before they become
/// readable, fails uploads on request, and survives the app being killed
final class CloudBackupEnableRecoveryFullAppUITests: XCTestCase {
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

    /// The background verifier keeps waiting while the cloud hides the accepted writes, then
    /// reaches Verified on its own once they become readable, with nothing tapped in between
    func testBackgroundVerificationWaitsForVisibilityThenVerifiesOnItsOwn() throws {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()

        // the verifier comes back to a hidden write; one pass checks the master key and the wallet
        // separately, so only a record checked twice shows a second pass
        control.waitUntil("the verifier never polled a pending upload twice") {
            $0.mostUploadStatusChecksOfOneRecord >= 2
        }
        control.waitForAppState("Settings should show the backup as confirming while its writes stay hidden") {
            $0.settingsRow == "confirming" && $0.awaitingUploadConfirmation
        }

        robot.openSettings()
        XCTAssertTrue(app.staticTexts["Cloud Backup Confirming"].waitForExistence(timeout: 15))

        // up to the release, the verifier has read none of the hidden backups, and the only passkey
        // calls are the enable's registration and its one confirmation
        let hidden = control.state
        XCTAssertEqual(hidden.count(.masterDownload), 0, "verification read the master key while it was hidden")
        XCTAssertEqual(hidden.walletDownloadCount, 0, "verification read a wallet while it was hidden")
        assertOnlyTheEnablesPasskeyCalls(hidden)

        control.releaseVisibility()

        // nothing is tapped here: the Settings row follows the background verifier
        XCTAssertTrue(app.staticTexts["Cloud Backup Enabled"].waitForExistence(timeout: 45))
        XCTAssertGreaterThan(control.state.count(.masterDownload), 0)

        robot.openCloudBackupDetail()
        robot.scrollToHittable(app.staticTexts["Backup verified"], initialWait: 15)
        control.assertNextActions("the verified backup should offer Verify and Turn Off", available: ["verify", "turnOff"])
        assertOnlyTheEnablesPasskeyCalls(control.state, "background confirmation asked for the passkey")
    }

    /// A fresh enable registers one passkey and confirms it once, and nothing else may prompt
    private func assertOnlyTheEnablesPasskeyCalls(
        _ state: CloudBackupUITestHostState,
        _ message: String = "a passkey prompt came from outside the enable",
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(state.count(.passkeyCreate), 1, message, file: file, line: line)
        XCTAssertEqual(state.count(.passkeyAuthenticate), 1, message, file: file, line: line)
        XCTAssertEqual(state.count(.passkeyDiscover), 0, message, file: file, line: line)
    }

    /// Killing the app after the cloud accepted the master key, before the upload returned, leaves
    /// a new process's normal startup to bring back the saved passkey's confirmation and finish
    /// the enable without creating another passkey
    func testKilledEnableRecoversThroughRelaunchWithoutNewPasskey() throws {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .freshEnable,
            namespaceResults: [.empty],
            holds: [.init(id: "master-upload", point: .masterUploadReturn)]
        ))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()
        control.waitForHold("master-upload")
        XCTAssertEqual(control.state.count(.passkeyCreate), 1)
        XCTAssertFalse(app.staticTexts["Cloud Backup enabled"].exists)

        robot.killAndRelaunch()

        let confirm = app.buttons["Confirm Passkey"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 30), "relaunch did not offer the saved passkey")
        XCTAssertFalse(app.staticTexts["Cloud Backup enabled"].exists)
        control.assertNextActions(
            "the relaunch should wait for the saved passkey's confirmation",
            body: "enabling",
            available: ["confirmSavedPasskey"]
        )

        // the new process shows the details step again, whose acknowledgements start unchecked
        if !confirm.isEnabled {
            robot.acceptCloudBackupAcknowledgements()
        }
        robot.scrollToHittable(confirm)
        confirm.tap()
        robot.finishOnboardingAfterCloudBackupEnabled()

        let state = control.state
        XCTAssertEqual(state.launches.count, 2, "expected one relaunch")
        XCTAssertEqual(state.count(.passkeyCreate), 1, "recovery created another passkey")
        XCTAssertGreaterThan(state.countInLatestLaunch(.passkeyAuthenticate), 0)
        XCTAssertGreaterThan(state.countInLatestLaunch(.masterUpload), 0)
        XCTAssertGreaterThan(state.countInLatestLaunch(.walletUpload), 0)

        // the recovered backup then confirms in the background like any other
        control.releaseVisibility()
        robot.openSettings()
        XCTAssertTrue(app.staticTexts["Cloud Backup Enabled"].waitForExistence(timeout: 45))
        control.assertNextActions("the recovered backup should offer Turn Off", available: ["turnOff"])
    }

    /// Killing the app while the passkey is still being registered leaves nothing a new process
    /// could confirm, so the relaunch discards the staged master key and a fresh enable starts over
    /// with one backup in the cloud
    func testKilledPasskeyRegistrationStartsOverOnRelaunch() throws {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .freshEnable,
            namespaceResults: [.empty],
            holds: [.init(id: "passkey-create", point: .passkeyCreate)]
        ))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()
        control.waitForHold("passkey-create")
        XCTAssertEqual(control.state.count(.masterUpload), 0)
        let journal = CloudBackupUITestControl.pendingEnableJournalKey
        XCTAssertTrue(try control.keychainContains(journal), "the enable never claimed its staged key")

        robot.killAndRelaunch()

        // the lifecycle reads disabled before restart recovery runs, so the journal's removal is
        // what shows the staged key was discarded
        try control.waitForKeychainItemRemoved(journal, "relaunch kept the interrupted enable's staged key")
        control.assertNextActions(
            "relaunch should discard the interrupted enable and offer a fresh one",
            body: "disabled",
            available: ["enable"]
        )

        // with no passkey to confirm, onboarding resumes either on the Cloud Backup details or on
        // its choice of backup method; runs have landed on both
        let enable = app.buttons["Enable Cloud Backup"]
        let backupStep = app.staticTexts["Back up your wallet"]
        let deadline = Date().addingTimeInterval(30)
        while !enable.exists, !backupStep.exists, Date() < deadline {
            _ = enable.waitForExistence(timeout: 1)
        }
        XCTAssertTrue(control.cloudNamespaces.isEmpty, "the interrupted enable wrote to the cloud")

        if !enable.exists, backupStep.exists {
            robot.openCloudBackupFromBackupStep()
        }
        XCTAssertTrue(enable.waitForExistence(timeout: 10), "relaunch did not offer a fresh enable")
        XCTAssertFalse(app.buttons["Confirm Passkey"].exists, "relaunch offered a passkey that was never registered")

        if !enable.isEnabled {
            robot.acceptCloudBackupAcknowledgements()
        }
        robot.tapEnableCloudBackup()
        robot.finishOnboardingAfterCloudBackupEnabled()

        let state = control.state
        XCTAssertEqual(state.launches.count, 2, "expected one relaunch")
        XCTAssertEqual(state.countInLatestLaunch(.passkeyCreate), 1, "the fresh enable should register its own passkey")
        XCTAssertEqual(state.countInLatestLaunch(.masterUpload), 1)
        XCTAssertEqual(control.cloudNamespaces.count, 1, "the cloud holds more than one backup")
    }

    /// Killing the app while it confirms the passkey it just registered keeps that passkey, so the
    /// relaunch offers Confirm Passkey and finishes the enable without registering another one
    func testKilledPasskeyConfirmationRecoversWithoutNewPasskey() throws {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .freshEnable,
            namespaceResults: [.empty],
            holds: [.init(id: "passkey-confirm", point: .passkeyAuthenticate)]
        ))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()
        control.waitForHold("passkey-confirm")
        XCTAssertEqual(control.state.count(.passkeyCreate), 1)
        XCTAssertEqual(control.state.count(.masterUpload), 0, "the enable uploaded before the passkey was confirmed")

        robot.killAndRelaunch()

        let confirm = app.buttons["Confirm Passkey"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 30), "relaunch did not offer the registered passkey")
        if !confirm.isEnabled {
            robot.acceptCloudBackupAcknowledgements()
        }
        robot.scrollToHittable(confirm)
        confirm.tap()
        robot.finishOnboardingAfterCloudBackupEnabled()
        control.assertNextActions("the confirmed enable should offer Turn Off", available: ["turnOff"])

        let state = control.state
        let registeredCredential = Data(CloudBackupUITestControl.passkeyCredentialId.utf8)
            .map { String(format: "%02x", $0) }
            .joined()
        XCTAssertEqual(state.launches.count, 2, "expected one relaunch")
        XCTAssertEqual(state.count(.passkeyCreate), 1, "recovery created another passkey")
        XCTAssertGreaterThan(state.countInLatestLaunch(.passkeyAuthenticate), 0)
        XCTAssertEqual(
            Set(state.authenticatedCredentialIds),
            [registeredCredential],
            "recovery confirmed another passkey"
        )
        XCTAssertEqual(control.cloudNamespaces.count, 1, "the cloud holds more than one backup")
    }

    /// Killing the app at the end of local promotion, once the new master key and passkey are
    /// committed and the cloud holds the backup, lets the relaunch finish the enable on its own,
    /// with no passkey prompt and nothing uploaded again
    func testKilledFinalPromotionFinishesOnRelaunch() throws {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .freshEnable,
            namespaceResults: [.empty],
            holds: [.init(id: "promotion", point: .pendingEnableJournalDelete)]
        ))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()
        control.waitForHold("promotion")
        let beforeKill = control.state
        XCTAssertEqual(beforeKill.count(.passkeyCreate), 1)
        XCTAssertGreaterThan(beforeKill.count(.walletUpload), 0, "promotion started before the wallet upload")
        let namespaces = control.cloudNamespaces
        XCTAssertEqual(namespaces.count, 1)

        // the writes are readable before the relaunch, so verification has nothing to repair and
        // any upload after it is the relaunch writing the backup again
        control.releaseVisibility()
        robot.killAndRelaunch()

        control.assertNextActions(
            "relaunch should finish the promoted enable without asking to confirm",
            available: ["turnOff"],
            unavailable: ["confirmSavedPasskey"]
        )
        XCTAssertFalse(
            app.buttons["Confirm Passkey"].waitForExistence(timeout: 5),
            "relaunch asked to confirm a passkey the promotion already committed"
        )
        robot.attachScreen(named: "after-promotion-relaunch")

        // the finished backup then confirms in the background like any other
        control.waitForAppState("the promoted backup never verified", timeout: 60) {
            !$0.awaitingUploadConfirmation && $0.verification == "verified"
        }

        let state = control.state
        XCTAssertEqual(state.launches.count, 2, "expected one relaunch")
        XCTAssertEqual(state.count(.passkeyCreate), 1, "the relaunch created another passkey")
        XCTAssertEqual(state.countInLatestLaunch(.passkeyAuthenticate), 0, "the relaunch asked for the passkey")
        XCTAssertEqual(state.countInLatestLaunch(.passkeyDiscover), 0, "the relaunch looked for a passkey")
        XCTAssertEqual(state.countInLatestLaunch(.masterUpload), 0, "the relaunch uploaded the master key again")
        XCTAssertEqual(state.countInLatestLaunch(.walletUpload), 0, "the relaunch uploaded a wallet again")
        XCTAssertEqual(control.cloudNamespaces, namespaces, "the relaunch wrote another backup")
    }

    /// A setup upload that fails is retried with the production Enable action, which reuses the
    /// passkey, master key and namespace the failed attempt staged and leaves one backup in the cloud
    func testFailedSetupUploadRetriesWithoutNewPasskey() throws {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .freshEnable,
            namespaceResults: [.empty],
            uploadFailures: [.init(kind: .wallet, attempt: 1, error: .uploadFailed)]
        ))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()
        let failure = robot.staticText(containing: "couldn't complete this cloud backup request")
        XCTAssertTrue(failure.waitForExistence(timeout: 30), "the failed upload was not reported")
        XCTAssertFalse(app.staticTexts["Cloud Backup enabled"].exists)

        let failedAttempt = control.state
        XCTAssertEqual(failedAttempt.count(.passkeyCreate), 1)
        let failedWallet = try XCTUnwrap(failedAttempt.uploads.last { $0.kind == .wallet })
        XCTAssertFalse(failedWallet.accepted)
        let namespace = failedWallet.namespace
        let walletRecordId = try XCTUnwrap(failedWallet.recordId)
        XCTAssertEqual(
            failedAttempt.uploads.filter { $0.kind == .master }.map(\.namespace),
            [namespace],
            "the failed attempt's master key went to another namespace"
        )

        // the enable is still running while this upload is held, so no background work made it
        try control.holdNextWalletUpload(id: "retry-wallet-upload")
        robot.tapEnableCloudBackup()
        control.waitForHold("retry-wallet-upload")
        XCTAssertFalse(app.staticTexts["Cloud Backup enabled"].exists)
        let retried = try XCTUnwrap(control.state.uploads.last { $0.kind == .wallet })
        XCTAssertEqual(retried.namespace, namespace, "the retry uploaded the wallet to a new namespace")
        XCTAssertEqual(retried.recordId, walletRecordId, "the retry uploaded another wallet record")
        XCTAssertTrue(retried.accepted)

        control.release("retry-wallet-upload")
        robot.finishOnboardingAfterCloudBackupEnabled()
        control.assertNextActions("the retried enable should offer Turn Off", available: ["turnOff"])

        // repeated writes to the same records are fine, a second namespace or record is not
        let state = control.state
        XCTAssertEqual(state.count(.passkeyCreate), 1, "retry created another passkey")
        XCTAssertEqual(Set(state.uploads.map(\.namespace)), [namespace], "an upload went to another namespace")
        XCTAssertEqual(control.cloudNamespaces, [namespace], "the cloud holds more than one backup")
        XCTAssertEqual(control.cloudWalletRecordIds(namespace: namespace), [walletRecordId])
        XCTAssertEqual(state.launches.count, 1)
    }
}
