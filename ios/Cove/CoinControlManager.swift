import SwiftUI

extension WeakReconciler: CoinControlManagerReconciler where Reconciler == CoinControlManager {}

@Observable final class CoinControlManager: AnyReconciler, CoinControlManagerReconciler {
    typealias Message = CoinControlManagerReconcileMessage
    typealias Action = CoinControlManagerAction

    private let logger = Log(id: "CoinControlManager")
    var rust: RustCoinControlManager

    private var sort: CoinControlListSort? = .some(.date(.descending))

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
                self.selected = $0
                self.dispatch(.notifySelectedUtxosChanged(Array($0)))
            }
        )
    }

    public init(_ rust: RustCoinControlManager) {
        self.rust = rust

        self.utxos = rust.utxos()
        self.unit = rust.unit()

        self.rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    public func buttonColor(_ key: CoinControlListSortKey) -> Color {
        let _ = self.sort
        return switch self.rust.buttonPresentation(button: key) {
        case .notSelected:
            .systemGray5
        case .selected:
            .blue
        }
    }

    public func buttonTextColor(_ key: CoinControlListSortKey) -> Color {
        let _ = self.sort
        return switch self.rust.buttonPresentation(button: key) {
        case .notSelected:
            .secondary.opacity(0.60)
        case .selected:
            .white
        }
    }

    public func buttonArrow(_ key: CoinControlListSortKey) -> String? {
        _ = self.sort
        return switch self.rust.buttonPresentation(button: key) {
        case .selected(.ascending):
            "arrow.up"
        case .selected(.descending):
            "arrow.down"
        case .notSelected:
            .none
        }
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

        let selectedUtxos = self.utxos.filter { self.selected.contains($0.id) }
        sfm.dispatch(.setCoinControlMode(selectedUtxos))
    }

    private func updateSendFlowManager() {
        guard let sfm = AppManager.shared.sendFlowManager else { return }
        self.updateSendFlowManagerTask?.cancel()
        self.updateSendFlowManagerTask = Task {
            try? await Task.sleep(for: .milliseconds(100))
            guard !Task.isCancelled else { return }
            let selectedUtxos = self.utxos.filter { self.selected.contains($0.id) }
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
        case let .updateTotalSelectedAmount(amount):
            updateSendFlowManager()
            withAnimation { self.totalSelected = amount }
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

    private let rustBridge = DispatchQueue(label: "cove.CoinControlManager.rustbridge", qos: .userInitiated)

    func reconcile(message: Message) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("reconcile: \(message)")
            apply(message)
        }
    }

    func reconcileMany(messages: [Message]) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("reconcile_messages: \(messages)")
            messages.forEach { self.apply($0) }
        }
    }

    public func dispatch(action: Action) {
        dispatch(action)
    }

    public func dispatch(_ action: Action) {
        rustBridge.async {
            self.logger.debug("dispatch: \(action)")
            self.rust.dispatch(action: action)
        }
    }
}
