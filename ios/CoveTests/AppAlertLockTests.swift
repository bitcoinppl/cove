@testable import Cove
import CoveCore
import SwiftUI
import XCTest

/// An app alert belongs to the unlocked session and the wallet mode that raised it
///
/// Uses the real auth and app managers and the real alert host, since what matters is how the app
/// lock and a decoy unlock reach an alert that is still queued or open
@MainActor
final class AppAlertLockTests: XCTestCase {
    private static let mainPin = "975310"
    private static let decoyPin = "864208"
    private static let newTrickPin = "531975"

    /// The lock is a SwiftUI overlay and UIKit presents an alert above it, so an alert queued behind
    /// a closing sheet could open over the PIN screen, such as a Cloud Backup failure for main wallets
    func testAnAlertWaitsBehindTheLockUntilThePinOpensTheApp() async throws {
        executionTimeAllowance = 120
        let auth = try await setUpPinLock()

        let model = AlertHostModel()
        let host = UIHostingController(rootView: AlertHostRoot(model: model))
        let display = try HostedWindow(root: host)
        defer { display.close() }

        auth.lock(reason: .background)
        model.alert = .init(TestAlert())
        try await settle()
        XCTAssertNil(host.presentedViewController, "the alert showed over the PIN screen")

        let unlock = await auth.handleAndReturnUnlockMode(Self.mainPin)
        XCTAssertEqual(unlock, .main)
        try await waitUntil("the alert must show once the PIN opens the app") {
            host.presentedViewController is UIAlertController
        }
    }

    /// The decoy PIN must open a Cove with no hint of the main wallets, so an alert they raised is
    /// gone by the time the lock opens, not when the mode change reaches the app a moment later
    func testTheDecoyPinOpensTheAppWithoutTheMainModeAlert() async throws {
        executionTimeAllowance = 120
        let auth = try await setUpPinLock()
        let app = AppManager.shared

        try auth.rust.setDecoyPin(pin: Self.decoyPin)
        addTeardownBlock { @MainActor in
            if auth.isInDecoyMode() { auth.switchToMainMode() }
            auth.dispatch(action: .disableDecoyPin)
        }

        app.alertState = .init(.general(title: "Cloud Backup", message: "Verification failed"))
        auth.lock(reason: .background)

        let decoyUnlock = await auth.handleAndReturnUnlockMode(Self.decoyPin)
        XCTAssertEqual(decoyUnlock, .decoy)
        XCTAssertNil(app.alertState, "the decoy PIN opened the app with the main-mode alert still up")

        // the main PIN opens the mode the app locked in, so an alert raised there stays to be read
        auth.switchToMainMode()
        try await settle()

        let alert = TaggedItem(AppAlertState.general(title: "Cloud Backup", message: "Verification failed"))
        app.alertState = alert
        auth.lock(reason: .background)

        let mainUnlock = await auth.handleAndReturnUnlockMode(Self.mainPin)
        XCTAssertEqual(mainUnlock, .main)
        XCTAssertEqual(app.alertState, alert, "the main PIN dropped an alert raised in main mode")
    }

    /// Builds before Change PIN refused trick PINs could save the app PIN equal to the decoy PIN,
    /// which then opened the main wallets; the unlock that finds it turns the decoy PIN off with no
    /// alert at the lock, where anyone could read it, and Security Settings keeps a notice instead
    func testAnAppPinEqualToTheDecoyPinTurnsItOffWithASettingsNotice() async throws {
        executionTimeAllowance = 120
        let auth = try await setUpPinLock()
        let app = AppManager.shared
        let duplicateHash = try hash(of: Self.decoyPin, then: Self.mainPin, auth: auth)

        try auth.rust.setDecoyPin(pin: Self.decoyPin)
        addTeardownBlock { @MainActor in
            if auth.isInDecoyMode() { auth.switchToMainMode() }
            auth.dispatch(action: .disableDecoyPin)
            auth.dispatch(action: .dismissTrickPinNotice)
        }
        try Database().globalConfig().setHashedPinCode(hashedPinCode: duplicateHash)

        auth.lock(reason: .background)
        let unlock = await auth.handleAndReturnUnlockMode(Self.decoyPin)
        XCTAssertEqual(unlock, .main, "the duplicate PIN should open the main wallets")
        XCTAssertNil(app.alertState, "the unlock raised an alert anyone holding the phone could read")

        XCTAssertFalse(auth.rust.isDecoyPinEnabled(), "the duplicate decoy PIN is still saved")
        try await waitUntil("Settings still shows the duplicate decoy PIN on") { !auth.isDecoyPinEnabled }
        try await waitUntil("Security Settings never got the notice") { auth.trickPinNotice == .decoy }

        // the next unlock finds nothing new and leaves the notice up
        auth.lock(reason: .background)
        let again = await auth.handleAndReturnUnlockMode(Self.decoyPin)
        XCTAssertEqual(again, .main)
        XCTAssertNil(app.alertState)
        try await settle()
        XCTAssertEqual(auth.trickPinNotice, .decoy, "the notice went away before the user answered it")

        // a new decoy PIN answers the notice and works again
        try auth.rust.setDecoyPin(pin: Self.newTrickPin)
        try await waitUntil("the notice still asks for a new decoy PIN") { auth.trickPinNotice == nil }
        auth.lock(reason: .background)
        let decoy = await auth.handleAndReturnUnlockMode(Self.newTrickPin)
        XCTAssertEqual(decoy, .decoy, "the new decoy PIN didn't open decoy mode")
    }

    /// The same for the wipe data PIN, which never wiped while it equaled the app PIN; the notice
    /// stays until the user dismisses it
    func testAnAppPinEqualToTheWipeDataPinTurnsItOffUntilDismissed() async throws {
        executionTimeAllowance = 120
        let auth = try await setUpPinLock()
        let app = AppManager.shared
        let duplicateHash = try hash(of: Self.decoyPin, then: Self.mainPin, auth: auth)

        try auth.rust.setWipeDataPin(pin: Self.decoyPin)
        addTeardownBlock { @MainActor in
            auth.dispatch(action: .disableWipeDataPin)
            auth.dispatch(action: .dismissTrickPinNotice)
        }
        try Database().globalConfig().setHashedPinCode(hashedPinCode: duplicateHash)

        auth.lock(reason: .background)
        let unlock = await auth.handleAndReturnUnlockMode(Self.decoyPin)
        XCTAssertEqual(unlock, .main, "the duplicate PIN should open the main wallets")
        XCTAssertNil(app.alertState, "the unlock raised an alert anyone holding the phone could read")

        XCTAssertFalse(auth.rust.isWipeDataPinEnabled(), "the duplicate wipe data PIN is still saved")
        try await waitUntil("Settings still shows the duplicate wipe data PIN on") { !auth.isWipeDataPinEnabled }
        try await waitUntil("Security Settings never got the notice") { auth.trickPinNotice == .wipeData }

        auth.dispatch(action: .dismissTrickPinNotice)
        try await waitUntil("dismissing kept the notice") { auth.trickPinNotice == nil }
        XCTAssertNil(auth.rust.trickPinNotice(), "dismissing kept the saved notice")
    }

    /// The app PIN hash for `pin`, saved through Change PIN's own path, leaving `current` as the PIN
    private func hash(of pin: String, then current: String, auth: AuthManager) throws -> String {
        try auth.setPin(pin)
        let hash = try Database().globalConfig().hashedPinCode()
        try auth.setPin(current)
        return hash
    }

    /// Sets a PIN on an app with no lock and removes it after the test
    private func setUpPinLock() async throws -> AuthManager {
        try await waitForBootstrap()
        let auth = AuthManager.shared
        let app = AppManager.shared

        // the test sets and removes its own PIN, so never run it over a credential someone relies on
        try XCTSkipUnless(auth.type == .none, "an app lock is already set up on this simulator")
        try XCTSkipUnless(!auth.isInDecoyMode(), "the simulator is in decoy mode")

        // turning off the PIN keeps the decoy PIN, which the test would replace and then delete
        try XCTSkipUnless(!auth.rust.isDecoyPinEnabled(), "a decoy PIN is already saved on this simulator")

        try auth.setPin(Self.mainPin)
        addTeardownBlock { @MainActor in
            app.alertState = nil
            auth.dispatch(action: .disablePin)
            auth.unlockIfAuthDisabled()
        }

        try await waitUntil("setting the PIN must enable the app lock") { auth.type == .pin }
        return auth
    }

    private func settle() async throws {
        try await Task.sleep(for: .milliseconds(800))
    }

    private func waitUntil(
        _ message: String,
        timeout: TimeInterval = 15,
        file: StaticString = #filePath,
        line: UInt = #line,
        _ condition: () -> Bool
    ) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() {
            guard Date() < deadline else {
                XCTFail(message, file: file, line: line)
                throw CancellationError()
            }

            try await Task.sleep(for: .milliseconds(50))
        }
    }

    private func waitForBootstrap() async throws {
        do {
            _ = try await bootstrap()
        } catch AppInitError.AlreadyCalled(_) {}

        // the app and the test can start bootstrap at the same time
        let deadline = Date().addingTimeInterval(15)
        while bootstrapProgress() != .complete {
            guard Date() < deadline else {
                XCTFail("App bootstrap did not complete before the alert lock test")
                throw CancellationError()
            }

            try await Task.sleep(for: .milliseconds(100))
        }
    }
}

private struct TestAlert: TaggedAlertPresentable {
    func alert(context _: Void) -> AnyAlertBuilder {
        AlertBuilder(title: "Cloud Backup", message: "Verification failed") {
            Button("OK") {}
        }
        .eraseToAny()
    }
}

private struct TestSheet: TaggedSheetPresentable {
    func sheet(context _: Void) -> AnyView {
        AnyView(EmptyView())
    }
}

@Observable
private final class AlertHostModel {
    var alert: TaggedItem<TestAlert>?
    var sheet: TaggedItem<TestSheet>?
}

private struct AlertHostRoot: View {
    @Bindable var model: AlertHostModel

    var body: some View {
        Color.clear.presentingSheetAndAlert(
            sheet: $model.sheet,
            sheetContext: (),
            alert: $model.alert,
            alertContext: ()
        )
    }
}

private final class HostedWindow {
    let window: UIWindow
    private let previousKeyWindow: UIWindow?

    init(root: UIViewController) throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        previousKeyWindow = scene.keyWindow
        window = UIWindow(windowScene: scene)
        window.frame = scene.coordinateSpace.bounds
        window.rootViewController = root
        window.makeKeyAndVisible()
    }

    func close() {
        window.isHidden = true
        window.rootViewController = nil
        previousKeyWindow?.makeKey()
    }
}
