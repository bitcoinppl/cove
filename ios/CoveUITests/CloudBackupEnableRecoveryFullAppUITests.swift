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
    /// passkey and master key the failed attempt staged
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
        XCTAssertEqual(control.state.count(.passkeyCreate), 1)
        XCTAssertEqual(control.state.count(.walletUpload), 1)

        robot.tapEnableCloudBackup()
        robot.finishOnboardingAfterCloudBackupEnabled()

        let state = control.state
        XCTAssertEqual(state.count(.passkeyCreate), 1, "retry created another passkey")
        // the background verifier may upload again while the cloud hides the writes, so only
        // the retry's own upload is certain
        XCTAssertGreaterThanOrEqual(state.count(.walletUpload), 2, "retry did not upload the wallet again")
        XCTAssertEqual(state.launches.count, 1)
    }
}
