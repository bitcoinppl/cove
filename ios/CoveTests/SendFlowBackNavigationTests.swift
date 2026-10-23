@testable import Cove
import CoveCore
import SwiftUI
import XCTest

/// Regression for #927: going back from Send at the largest text size pinned the main thread
/// in lazy layout until the watchdog killed the app
///
/// Run with `-test-timeouts-enabled YES` so a hang fails the test instead of stalling the run
@MainActor
final class SendFlowBackNavigationTests: XCTestCase {
    func testBackFromSendAtLargestAccessibilitySizeFinishes() async throws {
        executionTimeAllowance = 60
        try await waitForBootstrap()

        let fixture = BackNavigationFixture()
        defer { fixture.close() }

        let probe = SendNavigationProbe()
        let host = UIHostingController(rootView: SendNavigationTestRoot(probe: probe, manager: fixture.manager)
            .environment(fixture.app)
            .environment(AuthManager.shared)
            .environment(fixture.manager)
            .environment(fixture.sendManager)
            .environment(fixture.presenter)
            .environment(\.dynamicTypeSize, .accessibility5))
        host.traitOverrides.preferredContentSizeCategory = .accessibilityExtraExtraExtraLarge

        let display = try HostedNavigationWindow(root: host)
        defer { display.close() }

        host.view.layoutIfNeeded()
        try await Task.sleep(for: .seconds(1))
        let navigation = try XCTUnwrap(findNavigationController(in: host))

        // the hang needs a few round trips, and Send must finish focusing before Back
        for iteration in 0 ..< 3 {
            withAnimation { probe.path.append(iteration) }
            try await waitUntil("Send must be pushed and focus the amount") {
                navigation.viewControllers.count == 2
                    && navigation.transitionCoordinator == nil
                    && fixture.presenter.focusField == .amount
            }

            withAnimation { probe.path.removeLast() }
            try await waitUntil("Back must restore the wallet and finish its transition") {
                navigation.viewControllers.count == 1 && navigation.transitionCoordinator == nil
            }

            host.view.layoutIfNeeded()
        }
    }

    /// Polls on the main actor so a slow simulator gets time, while a pinned main thread still
    /// fails through `executionTimeAllowance`
    private func waitUntil(
        _ message: String,
        timeout: TimeInterval = 10,
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
                XCTFail("App bootstrap did not complete before layout testing")
                throw CancellationError()
            }

            try await Task.sleep(for: .milliseconds(50))
        }
    }

    private func findNavigationController(in controller: UIViewController) -> UINavigationController? {
        if let navigation = controller as? UINavigationController { return navigation }
        for child in controller.children {
            if let navigation = findNavigationController(in: child) { return navigation }
        }

        return nil
    }
}

/// Keeps layout and focus behavior real while removing network and funding requirements
private final class LayoutOnlySendManager: SendFlowRustManaging {
    private let id: WalletId

    init(walletId: WalletId) { id = walletId }
    func walletId() -> WalletId { id }
    func enteringFiatAmount() -> String { "" }
    func sendAmountFiat() -> String { "" }
    func sendAmountBtc() -> String { "" }
    func totalSpentInFiat() -> String { "" }
    func totalSpentInBtc() -> String { "" }
    func totalFeeString() -> String? { nil }
    func listenForUpdates(reconciler _: SendFlowManagerReconciler) {}
    func validateAddress(displayAlert _: Bool) -> Bool { false }
    func validateAmount(displayAlert _: Bool) -> Bool { false }
    func waitForInit() async -> Bool { true }
    func amountExceedsBalance() -> Bool { false }
    func sanitizeBtcEnteringAmount(oldValue _: String, newValue _: String) -> String? { nil }
    func sanitizeFiatEnteringAmount(oldValue _: String, newValue _: String) -> String? { nil }
    func utxos() -> [Utxo]? { nil }
    func maxSendMinusFees() -> Amount? { nil }
    func maxSendMinusFeesAndSmallUtxo() -> Amount? { nil }
    func amount() -> Amount { Amount.fromSat(sats: 0) }
    func dispatch(action _: SendFlowManagerAction) {}

    func getCustomFeeOption(
        feeRate _: FeeRate,
        feeSpeed _: FeeSpeed
    ) async throws -> FeeRateOptionWithTotalFee {
        throw CancellationError()
    }
}

@MainActor
@Observable
private final class SendNavigationProbe {
    var path: [Int] = []
}

private struct SendNavigationTestRoot: View {
    @Bindable var probe: SendNavigationProbe
    let manager: WalletManager

    var body: some View {
        NavigationStack(path: $probe.path) {
            SelectedWalletScreen(manager: manager)
                .navigationDestination(for: Int.self) { _ in
                    SendFlowSetAmountScreen(id: manager.id)
                }
        }
        // the app root injects the safe area the wallet screen lays out with
        .capturesSafeAreaInsets()
    }
}

@MainActor
private final class BackNavigationFixture {
    let app = AppManager.shared
    let manager: WalletManager
    let presenter: SendFlowPresenter
    let sendManager: SendFlowManager
    private let previousLockState = AuthManager.shared.lockState
    private let previousSelection = Database().globalConfig().selectedWallet()

    init() {
        var metadata = walletMetadataPreview()
        metadata.verified = true
        manager = WalletManager(preview: .only, metadata)

        // populate the real lazy list without requiring a funded simulator wallet
        manager.loadState = .loaded(transactionsPreviewNew(confirmed: 32, unconfirmed: 0))

        // a real backing handle is required because Balance equality runs in Rust
        manager.balance = LayoutBalance(unsafeFromHandle: Balance.zero().uniffiCloneHandle())
        AuthManager.shared.lockState = .unlocked
        presenter = SendFlowPresenter(app: app, manager: manager)
        sendManager = SendFlowManager(LayoutOnlySendManager(walletId: manager.id), presenter: presenter)
    }

    func close() {
        sendManager.close()
        manager.close()
        AuthManager.shared.lockState = previousLockState
        if let previousSelection {
            try? Database().globalConfig().selectWallet(id: previousSelection)
        } else {
            try? Database().globalConfig().clearSelectedWallet()
        }
    }
}

private final class LayoutBalance: Balance, @unchecked Sendable {
    override func spendable() -> Amount { Amount.fromSat(sats: 1_436_900) }
}

@MainActor
private final class HostedNavigationWindow {
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
