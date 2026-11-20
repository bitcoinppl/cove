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

    /// Sets a PIN on an app with no lock and removes it after the test
    private func setUpPinLock() async throws -> AuthManager {
        try await waitForBootstrap()
        let auth = AuthManager.shared
        let app = AppManager.shared

        // the test sets and removes its own PIN, so never run it over a credential someone relies on
        try XCTSkipUnless(auth.type == .none, "an app lock is already set up on this simulator")
        try XCTSkipUnless(!auth.isInDecoyMode(), "the simulator is in decoy mode")

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
