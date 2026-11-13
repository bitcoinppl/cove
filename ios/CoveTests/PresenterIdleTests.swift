@testable import Cove
import UIKit
import XCTest

final class PresenterIdleTests: XCTestCase {
    private let windows = StubWindows()

    override func tearDown() {
        MainActor.assumeIsolated {
            windows.close()
        }

        super.tearDown()
    }

    @MainActor
    func testSettledSheetChainIsIdle() {
        let root = StubController()
        root.stubPresented = StubController()

        XCTAssertTrue(PresenterIdle.isIdle(from: root))
        XCTAssertTrue(PresenterIdle.isIdle(from: nil))
    }

    @MainActor
    func testAlertAnywhereInChainIsBusy() {
        let root = StubController()
        let sheet = StubController()
        root.stubPresented = sheet
        sheet.stubPresented = UIAlertController(title: nil, message: nil, preferredStyle: .alert)

        XCTAssertFalse(PresenterIdle.isIdle(from: root))
    }

    @MainActor
    func testDismissingSheetIsBusy() {
        let root = StubController()
        let sheet = StubController()
        sheet.stubBeingDismissed = true
        root.stubPresented = sheet

        XCTAssertFalse(PresenterIdle.isIdle(from: root))
    }

    func testSettleNeedsConsecutiveReadyChecks() {
        let now = ContinuousClock.now
        var settle = PresenterIdle.Settle<String>(deadline: now + .seconds(1))

        let steps = ["first", nil, "second", "third"].map { settle.record($0, at: now) }

        XCTAssertEqual(steps, [.poll, .poll, .poll, .settled("third")])
    }

    func testSettleTimesOutWithoutConsecutiveReadyChecks() {
        let start = ContinuousClock.now
        var settle = PresenterIdle.Settle<String>(deadline: start + .milliseconds(100))

        XCTAssertEqual(settle.record(nil, at: start), .poll)
        XCTAssertEqual(settle.record("ready", at: start + .milliseconds(100)), .timedOut)
    }

    @MainActor
    func testOnlyAVisibleKeyWindowOfAForegroundActiveSceneIsReady() throws {
        let window = try windows.make()

        XCTAssertIdentical(
            PresenterIdle.readyWindow(in: [StubScene(activationState: .foregroundActive, windows: [window])]),
            window
        )

        for state in [UIScene.ActivationState.foregroundInactive, .background, .unattached] {
            XCTAssertNil(PresenterIdle.readyWindow(in: [StubScene(activationState: state, windows: [window])]))
        }

        let notKey = try windows.make(isKey: false)
        XCTAssertNil(PresenterIdle.readyWindow(in: [StubScene(activationState: .foregroundActive, windows: [notKey])]))

        let hidden = try windows.make(isHidden: true)
        XCTAssertNil(PresenterIdle.readyWindow(in: [StubScene(activationState: .foregroundActive, windows: [hidden])]))
    }

    @MainActor
    func testWaitReturnsTheReadyWindow() async throws {
        let window = try windows.make()

        let readiness = await PresenterIdle.waitUntilReady(
            timeout: .seconds(5),
            scenes: { [StubScene(activationState: .foregroundActive, windows: [window])] },
            isAppLocked: { false }
        )

        guard case let .ready(readyWindow) = readiness else {
            XCTFail("expected the active key window to be ready")
            return
        }

        XCTAssertIdentical(readyWindow, window)
    }

    @MainActor
    func testWaitIsNotReadyAfterTheTimeoutWhenTheSceneIsInactive() async throws {
        let window = try windows.make()

        let readiness = await PresenterIdle.waitUntilReady(
            timeout: .milliseconds(100),
            scenes: { [StubScene(activationState: .foregroundInactive, windows: [window])] },
            isAppLocked: { false }
        )

        guard case .notReady = readiness else {
            XCTFail("an inactive scene must not be ready")
            return
        }
    }

    @MainActor
    func testWaitIsNotReadyAfterTheTimeoutWhenThePresenterStaysBusy() async throws {
        let root = StubController()
        root.stubPresented = UIAlertController(title: nil, message: nil, preferredStyle: .alert)
        let window = try windows.make(root: root)

        let readiness = await PresenterIdle.waitUntilReady(
            timeout: .milliseconds(100),
            scenes: { [StubScene(activationState: .foregroundActive, windows: [window])] },
            isAppLocked: { false }
        )

        guard case .notReady = readiness else {
            XCTFail("a presenter still busy at the timeout must not be ready")
            return
        }
    }

    /// The PIN lock is a SwiftUI overlay inside an active window with an idle controller chain, so
    /// only the lock state tells it apart from a ready presenter. A locked app is not ready at once
    /// and does not hold the request until the user unlocks it
    @MainActor
    func testLockedAppIsNotReadyAtOnceEvenWhenItUnlocksDuringTheWait() async throws {
        let window = try windows.make()
        let lock = MainActorFlag(true)
        let clock = ContinuousClock()
        let started = clock.now

        let unlock = Task { @MainActor in
            try await Task.sleep(for: .milliseconds(100))
            lock.value = false
        }

        let readiness = await PresenterIdle.waitUntilReady(
            timeout: .seconds(5),
            scenes: { [StubScene(activationState: .foregroundActive, windows: [window])] },
            isAppLocked: { lock.value }
        )
        unlock.cancel()

        guard case .notReady = readiness else {
            XCTFail("a locked app must not be ready, even after a later unlock")
            return
        }

        XCTAssertLessThan(clock.now - started, .seconds(1), "a locked app must not wait out the timeout")
    }

    @MainActor
    func testIsStillReadyOnlyWhileTheSettledWindowIsReadyAndTheAppUnlocked() throws {
        let window = try windows.make()
        let active = StubScene(activationState: .foregroundActive, windows: [window])

        XCTAssertTrue(PresenterIdle.isStillReady(window, scenes: { [active] }, isAppLocked: { false }))
        XCTAssertFalse(PresenterIdle.isStillReady(window, scenes: { [active] }, isAppLocked: { true }))

        let inactive = StubScene(activationState: .foregroundInactive, windows: [window])
        XCTAssertFalse(PresenterIdle.isStillReady(window, scenes: { [inactive] }, isAppLocked: { false }))

        let otherWindow = try windows.make()
        let otherKey = StubScene(activationState: .foregroundActive, windows: [otherWindow, window])
        XCTAssertFalse(PresenterIdle.isStillReady(window, scenes: { [otherKey] }, isAppLocked: { false }))

        let root = StubController()
        root.stubPresented = UIAlertController(title: nil, message: nil, preferredStyle: .alert)
        let busy = try windows.make(root: root)
        let busyScene = StubScene(activationState: .foregroundActive, windows: [busy])
        XCTAssertFalse(PresenterIdle.isStillReady(busy, scenes: { [busyScene] }, isAppLocked: { false }))
    }
}

@MainActor
private final class MainActorFlag {
    var value: Bool

    init(_ value: Bool) {
        self.value = value
    }
}

struct StubScene: PresenterScene {
    let activationState: UIScene.ActivationState
    let windows: [UIWindow]
}

/// A window on the test host's scene whose key state the test sets, since only UIKit can make a
/// window key
final class StubWindow: UIWindow {
    var stubIsKey = true

    override var isKeyWindow: Bool {
        stubIsKey
    }
}

/// Windows a test made, hidden again when it ends so they never cover the host app
final class StubWindows: @unchecked Sendable {
    private var made: [UIWindow] = []

    @MainActor
    func make(
        isKey: Bool = true,
        isHidden: Bool = false,
        root: UIViewController? = nil
    ) throws -> UIWindow {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = StubWindow(windowScene: scene)
        window.stubIsKey = isKey
        // a default argument is evaluated outside the main actor, so the stub root is built here
        window.rootViewController = root ?? StubController()
        window.isHidden = isHidden
        made.append(window)
        return window
    }

    @MainActor
    func close() {
        for window in made {
            window.isHidden = true
            window.rootViewController = nil
        }

        made.removeAll()
    }
}

final class StubController: UIViewController {
    var stubPresented: UIViewController?
    var stubBeingDismissed = false

    override var presentedViewController: UIViewController? {
        stubPresented
    }

    override var isBeingDismissed: Bool {
        stubBeingDismissed
    }
}
