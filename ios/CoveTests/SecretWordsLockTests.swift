@testable import Cove
import CoveCore
import Darwin
import SwiftUI
import XCTest

/// The recovery words must not exist in the accessibility tree until the app lock is opened with a credential
///
/// Hosts the real screen under the real app lock and walks the accessibility tree, which is what
/// XCUITest and VoiceOver read, so covering the words with the lock overlay is not enough to pass
@MainActor
final class SecretWordsLockTests: XCTestCase {
    private static let testPin = "246813"
    private static let wrongPin = "135792"

    /// Published BIP39 test vector, never used for funds; distinct from the vector other tests import
    private static let testWords = [
        "legal", "winner", "thank", "year", "wave", "sausage",
        "worth", "useful", "legal", "winner", "thank", "yellow",
    ]

    override class func setUp() {
        super.setUp()
        enableAccessibilityAutomation()
    }

    func testWordsLoadOnlyAfterTheCorrectPinAndClearWhenTheAppLocksAgain() async throws {
        executionTimeAllowance = 120
        try await waitForBootstrap()

        let auth = AuthManager.shared

        // the test sets and removes its own PIN, so never run it over a credential someone relies on
        try XCTSkipUnless(auth.type == .none, "an app lock is already set up on this simulator")

        // the test deletes its wallet afterwards, so never run it over one it did not create
        guard let walletId = try importTestWallet() else {
            throw XCTSkip("the test wallet already exists on this simulator")
        }

        addTeardownBlock { await self.deleteTestWallet(walletId) }

        auth.dispatch(action: .setPin(Self.testPin))
        addTeardownBlock { @MainActor in
            auth.dispatch(action: .disablePin)
            auth.unlockIfAuthDisabled()
        }

        try await waitUntil("setting the PIN must enable the app lock") { auth.type == .pin }
        auth.lock(reason: .reauthentication)

        let host = UIHostingController(
            rootView: SecretWordsLockTestRoot(auth: auth, walletId: walletId)
                .environment(AppManager.shared)
                .environment(auth)
        )
        let display = try HostedWindow(root: host)
        defer { display.close() }

        // proves the walk reaches SwiftUI text, so an empty result below is not a blind spot
        try await waitUntil("the PIN cover must be in the accessibility tree") {
            accessibilityTokens(in: host.view).contains("enter cove pin")
        }
        try await settle()
        assertNoRecoveryWords(in: host.view, "while the PIN cover is up")

        let wrongPinResult = await auth.handleAndReturnUnlockMode(Self.wrongPin)
        XCTAssertEqual(wrongPinResult, .locked)
        try await settle()
        assertNoRecoveryWords(in: host.view, "after a wrong PIN")

        try await unlockAndWaitForWords(auth, in: host.view)

        auth.lock(reason: .background)
        try await settle()
        assertNoRecoveryWords(in: host.view, "after the app locks again")

        // a lock that a grace-period release reopens before the screen updates must still drop the words
        try await unlockAndWaitForWords(auth, in: host.view)
        leaveAndQuicklyReturn(auth)
        try await settle()
        assertNoRecoveryWords(in: host.view, "after a lock and grace-period unlock")
        XCTAssertEqual(auth.lockState, .locked, "the screen must ask for the credential again")

        // a credential the screen has not seen yet must not outlive a later lock that reopens without one,
        // so unlock through the lock screen binding and relock before SwiftUI runs another update
        auth.appLockState.wrappedValue = .unlocked
        leaveAndQuicklyReturn(auth)
        try await settle()
        assertNoRecoveryWords(in: host.view, "after a credential that a later lock outlived")
        XCTAssertEqual(auth.lockState, .locked, "the screen must ask for a fresh credential")

        try await unlockAndWaitForWords(auth, in: host.view)
    }

    /// A background lock released inside the foreground grace period, how the app opens without a
    /// credential while a PIN is set
    private func leaveAndQuicklyReturn(_ auth: AuthManager) {
        auth.lock(reason: .background)
        auth.releaseBackgroundLockOnForeground()
        XCTAssertEqual(auth.lockState, .unlocked, "the grace period must release a background-only lock")
    }

    private func unlockAndWaitForWords(_ auth: AuthManager, in view: UIView) async throws {
        // the same call the app lock makes when the PIN pad fills
        let unlockResult = await auth.handleAndReturnUnlockMode(Self.testPin)
        XCTAssertEqual(unlockResult, .main)
        try await waitUntil("the words must appear after the correct PIN") {
            let tokens = accessibilityTokens(in: view)
            return Self.testWords.allSatisfy { tokens.contains($0) }
        }
    }

    /// SwiftUI only builds its accessibility elements while an assistive client is attached, as
    /// XCUITest and VoiceOver are, so turn on the same automation flag they set before walking the tree
    private static func enableAccessibilityAutomation() {
        guard let handle = dlopen("/usr/lib/libAccessibility.dylib", RTLD_NOW),
              let symbol = dlsym(handle, "_AXSSetAutomationEnabled")
        else { return }

        typealias SetAutomationEnabled = @convention(c) (Bool) -> Void
        unsafeBitCast(symbol, to: SetAutomationEnabled.self)(true)
    }

    private func assertNoRecoveryWords(
        in view: UIView,
        _ moment: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let tokens = accessibilityTokens(in: view)
        let leaked = Set(Self.testWords).filter { tokens.contains($0) }
        XCTAssertTrue(leaked.isEmpty, "recovery words were in the accessibility tree \(moment): \(leaked.sorted())", file: file, line: line)
    }

    /// Every accessibility label and value under `root`, lowercased, plus each of their words
    private func accessibilityTokens(in root: UIView) -> Set<String> {
        var tokens = Set<String>()
        var visited = Set<ObjectIdentifier>()

        func record(_ text: String?) {
            guard let text, !text.isEmpty else { return }

            let lowered = text.lowercased()
            tokens.insert(lowered)
            lowered.components(separatedBy: CharacterSet.letters.inverted)
                .filter { !$0.isEmpty }
                .forEach { tokens.insert($0) }
        }

        func visit(_ object: NSObject) {
            guard visited.insert(ObjectIdentifier(object)).inserted else { return }

            record(object.accessibilityLabel)
            record(object.accessibilityValue)

            if let elements = object.accessibilityElements {
                elements.compactMap { $0 as? NSObject }.forEach(visit)
            } else {
                let count = object.accessibilityElementCount()
                if count != NSNotFound {
                    (0 ..< max(count, 0))
                        .compactMap { object.accessibilityElement(at: $0) as? NSObject }
                        .forEach(visit)
                }
            }

            if let view = object as? UIView { view.subviews.forEach(visit) }
        }

        visit(root)
        return tokens
    }

    /// Nil when the wallet already exists, since the test cannot know who else relies on it
    private func importTestWallet() throws -> WalletId? {
        do {
            return try ImportWalletManager().rust.importWallet(enteredWords: [Self.testWords]).id
        } catch ImportWalletError.WalletAlreadyExists {
            return nil
        }
    }

    private func deleteTestWallet(_ id: WalletId) async {
        // cleanup only, so a failure is logged instead of hiding the test result
        do {
            try await RustWalletManager(id: id).deleteWallet()
        } catch {
            Log.warn("failed to delete the secret words test wallet: \(error)")
        }
    }

    private func settle() async throws {
        try await Task.sleep(for: .milliseconds(500))
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
                XCTFail("App bootstrap did not complete before the secret words test")
                throw CancellationError()
            }

            try await Task.sleep(for: .milliseconds(50))
        }
    }
}

/// The app lock exactly as `CoveMainView` wires it, with the secret words screen as its content
private struct SecretWordsLockTestRoot: View {
    @Bindable var auth: AuthManager
    let walletId: WalletId

    var body: some View {
        LockView(
            lockType: auth.type,
            isPinCorrect: { await auth.handleAndReturnUnlockMode($0) != .locked },
            lockState: auth.appLockState
        ) {
            NavigationStack {
                SecretWordsScreen(id: walletId)
            }
        }
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
