import os
import SwiftUI

extension WeakReconciler: CoinControlManagerReconciler where Reconciler == CoinControlManager {}

private enum CoinControlManagerError: LocalizedError {
    case closed

    var errorDescription: String? {
        "Coin control manager is closed"
    }
}

@Observable final class CoinControlManager: AnyReconciler, CoinControlManagerReconciler {
    typealias Message = CoinControlManagerReconcileMessage
    typealias Action = CoinControlManagerAction

    private let logger = Log(id: "CoinControlManager")
    @ObservationIgnored
    private var rust: RustCoinControlManager?
    let id: WalletId
    @ObservationIgnored
    private let closeState = OSAllocatedUnfairLock(initialState: false)

    private(set) var sort: CoinControlListSort? = .some(.date(.descending))

    var search: String = ""
    var totalSelected = Amount.fromSat(sats: 0)
    var selected: Set<Utxo.ID> = []
    var utxos: [Utxo]
    var unit: Unit = .sat

    private var updateSendFlowManagerTask: Task<Void, Never>? = nil

    @ObservationIgnored
    var searchBinding: Binding<String> {
        Binding(
            get: { self.search },
            set: {
                if self.search != $0 { self.dispatch(.notifySearchChanged($0)) }
                self.search = $0
            }
        )
    }

    @ObservationIgnored
    var selectedBinding: Binding<Set<Utxo.ID>> {
        Binding(
            get: { self.selected },
            set: {
                let spendableIds = Set(self.utxos.filter(\.spendable).map(\.id))
                let selected = $0.intersection(spendableIds)
                self.selected = selected
                self.dispatch(.notifySelectedUtxosChanged(Array(selected)))
            }
        )
    }

    public init(_ rust: RustCoinControlManager) {
        self.rust = rust
        self.id = rust.id()

        self.utxos = rust.utxos()
        self.unit = rust.unit()

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    deinit {
        close()
    }

    func close() {
        guard markClosedIfNeeded() else { return }

        logger.debug("Closing CoinControlManager")
        updateSendFlowManagerTask?.cancel()
        updateSendFlowManagerTask = nil
        rust = nil
    }

    private func markClosedIfNeeded() -> Bool {
        closeState.withLock { isClosed in
            guard !isClosed else { return false }
            isClosed = true
            return true
        }
    }

    public func buttonArrow(_ key: CoinControlListSortKey) -> String? {
        _ = self.sort
        guard let rust else { return nil }

        return switch rust.buttonPresentation(button: key) {
        case .selected(.ascending):
            "arrow.up"
        case .selected(.descending):
            "arrow.down"
        case .notSelected:
            .none
        }
    }

    public func isSortSelected(_ key: CoinControlListSortKey) -> Bool {
        guard let rust else { return false }

        if case .selected = rust.buttonPresentation(button: key) { return true }
        return false
    }

    public func selectedUtxos() -> [Utxo] {
        rust?.selectedUtxos() ?? []
    }

    public var totalSelectedAmount: String {
        displayAmount(self.totalSelected)
    }

    public var totalSelectedSats: Int {
        Int(self.totalSelected.asSats())
    }

    public func continuePressed() {
        guard let sfm = AppManager.shared.sendFlowManager else { return }
        self.updateSendFlowManagerTask?.cancel()
        self.updateSendFlowManagerTask = nil

        let selectedUtxos = self.utxos.filter { $0.spendable && self.selected.contains($0.id) }
        sfm.dispatch(.setCoinControlMode(selectedUtxos))
    }

    private func updateSendFlowManager() {
        guard let sfm = AppManager.shared.sendFlowManager else { return }
        self.updateSendFlowManagerTask?.cancel()
        self.updateSendFlowManagerTask = Task {
            try? await Task.sleep(for: .milliseconds(100))
            guard !Task.isCancelled else { return }
            let selectedUtxos = self.utxos.filter { $0.spendable && self.selected.contains($0.id) }
            sfm.dispatch(.setCoinControlMode(selectedUtxos))
        }
    }

    private func apply(_ message: Message) {
        switch message {
        case let .updateSort(sort):
            withAnimation { self.sort = sort }
        case .clearSort:
            withAnimation { self.sort = .none }
        case let .updateUtxos(utxos):
            withAnimation { self.utxos = utxos }
        case let .updateSearch(search):
            withAnimation { self.search = search }
        case let .updateSelectedUtxos(utxos: selected, totalSelected):
            updateSendFlowManager()
            self.selected = Set(selected)
            withAnimation { self.totalSelected = totalSelected }
        case let .updateUnit(unit):
            withAnimation { self.unit = unit }
        }
    }

    func displayAmount(_ amount: Amount, showUnit: Bool = true) -> String {
        switch (unit, showUnit) {
        case (.btc, true):
            amount.btcStringWithUnit()
        case (.btc, false):
            amount.btcString()
        case (.sat, true):
            amount.satsStringWithUnit()
        case (.sat, false):
            amount.satsString()
        }
    }

    func reloadLabels() async {
        guard let rust else { return }

        await rust.reloadLabels()
    }

    func setSpendability(_ spendable: Bool, for outpoint: OutPoint) async throws {
        guard let rust else { throw CoinControlManagerError.closed }

        try await rust.setUtxoSpendability(outpoint: outpoint, spendable: spendable)
    }

    private let rustBridge = DispatchQueue(label: "cove.CoinControlManager.rustbridge", qos: .userInitiated)

    func reconcile(message: Message) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.rust != nil else { return }
            logger.debug("reconcile: \(message)")
            apply(message)
        }
    }

    func reconcileMany(messages: [Message]) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.rust != nil else { return }
            logger.debug("reconcile_messages: \(messages)")
            messages.forEach { self.apply($0) }
        }
    }

    public func dispatch(action: Action) {
        dispatch(action)
    }

    public func dispatch(_ action: Action) {
        rustBridge.async { [weak self] in
            guard let self, let rust = self.rust else { return }

            self.logger.debug("dispatch: \(action)")
            rust.dispatch(action: action)
        }
    }
}
