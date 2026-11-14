@testable import Cove
import CoveCore
import os
import XCTest

/// Rust counts every address edit as a newer address and drops a wallet pick still on its way,
/// so the address field writing back the text Rust just set must never reach Rust
@MainActor
final class SendFlowAddressEntryTests: XCTestCase {
    func testWritingBackTheAddressRustSetKeepsAWalletPickAlive() async throws {
        let wallet = WalletManager(preview: .only)
        let rust = RecordingSendManager(walletId: wallet.id)
        let presenter = SendFlowPresenter(app: AppManager.shared, manager: wallet)
        let manager = SendFlowManager(rust, presenter: presenter)
        defer {
            manager.close()
            wallet.close()
        }

        // picking a wallet empties the field while Rust fetches that wallet's address
        manager.apply(.updateEnteringAddress("bc1qearlierrecipient"))
        manager.apply(.updateEnteringAddress(""))

        // the field's whitespace cleanup writes the same text back
        manager.enteringAddress.wrappedValue = ""

        let typed = "bc1qtypedbytheuser"
        manager.enteringAddress.wrappedValue = typed

        // dispatches run in order, so the echo would be recorded before the typed address
        let deadline = Date().addingTimeInterval(5)
        while rust.enteredAddresses.isEmpty, Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }

        XCTAssertEqual(rust.enteredAddresses, [typed])
    }
}

/// Records the addresses the send flow tells Rust the user entered
private final class RecordingSendManager: SendFlowRustManaging {
    private let id: WalletId
    private let entered = OSAllocatedUnfairLock(initialState: [String]())

    var enteredAddresses: [String] {
        entered.withLock { $0 }
    }

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
    func transferDestinations() -> [TransferDestination] { [] }

    func dispatch(action: SendFlowManagerAction) {
        guard case let .notifyEnteringAddressChanged(address) = action else { return }
        entered.withLock { $0.append(address) }
    }

    func getCustomFeeOption(
        feeRate _: FeeRate,
        feeSpeed _: FeeSpeed
    ) async throws -> FeeRateOptionWithTotalFee {
        throw CancellationError()
    }
}
