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
        CloudBackupHostControl.attachAppLogIfFailed(to: self)
        super.tearDown()
    }

    /// The background verifier keeps waiting while the cloud hides the accepted writes, then
    /// reaches Verified on its own once they become readable, with nothing tapped in between
    func testBackgroundVerificationWaitsForVisibilityThenVerifiesOnItsOwn() throws {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()

        // the verifier has polled the hidden writes more than once and read none of them
        control.waitUntil("the verifier never polled the pending uploads") {
            $0.count(.uploadStatus) >= 2
        }
        XCTAssertEqual(control.state.count(.masterDownload), 0)
        XCTAssertEqual(control.state.count(.passkeyCreate), 1)

        robot.openSettings()
        XCTAssertTrue(app.staticTexts["Cloud Backup Confirming"].waitForExistence(timeout: 15))
        let authenticationsBeforeVisibility = control.state.count(.passkeyAuthenticate)

        control.releaseVisibility()

        // nothing is tapped here: the Settings row follows the background verifier
        XCTAssertTrue(app.staticTexts["Cloud Backup Enabled"].waitForExistence(timeout: 45))
        XCTAssertGreaterThan(control.state.count(.masterDownload), 0)
        XCTAssertEqual(
            control.state.count(.passkeyAuthenticate),
            authenticationsBeforeVisibility,
            "background confirmation must not ask for the passkey"
        )

        robot.openCloudBackupDetail()
        robot.scrollToHittable(app.staticTexts["Backup verified"], initialWait: 15)
        XCTAssertEqual(control.state.count(.passkeyCreate), 1)
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

        // repeated writes to the same records are fine, a second namespace or record is not
        let state = control.state
        XCTAssertEqual(state.count(.passkeyCreate), 1, "retry created another passkey")
        XCTAssertEqual(Set(state.uploads.map(\.namespace)), [namespace], "an upload went to another namespace")
        XCTAssertEqual(control.cloudNamespaces, [namespace], "the cloud holds more than one backup")
        XCTAssertEqual(control.cloudWalletRecordIds(namespace: namespace), [walletRecordId])
        XCTAssertEqual(state.launches.count, 1)
    }
}
