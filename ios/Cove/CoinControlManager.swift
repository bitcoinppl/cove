import SwiftUI

extension WeakReconciler: CoinControlManagerReconciler where Reconciler == CoinControlManager {}

@Observable final class CoinControlManager: AnyReconciler, CoinControlManagerReconciler {
    typealias Message = CoinControlManagerReconcileMessage
    typealias Action = CoinControlManagerAction

    private let logger = Log(id: "CoinControlManager")
    var rust: RustCoinControlManager

    private var sort: CoinControlListSort? = .some(.date(.descending))

    var search: String = ""
    var selected: Set<Utxo.ID> = []
    var utxos: [Utxo]
    var unit: Unit = .sat

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
        let _ = self.sort
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
        let _ = self.selected
        let total = rust.totalSelectedAmount()
        return displayAmount(total)
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
        case let .updateSelectedUtxos(selected):
            self.selected = Set(selected)
        case let .updateUnit(unit):
            withAnimation { self.unit = unit }
        }
    }

    func displayAmount(_ amount: Amount) -> String {
        switch unit {
        case .btc:
            amount.btcStringWithUnit()
        case .sat:
            amount.satsStringWithUnit()
        }
    }

    private let rustBridge = DispatchQueue(label: "cove.CoinControlManager.rustbridge", qos: .userInitiated)

    func reconcile(message: Message) {
        rustBridge.async { [weak self] in
            guard let self else {
                Log.error("CoinControlManager no longer available")
                return
            }

            logger.debug("reconcile: \(message)")
            DispatchQueue.main.async { [self] in
                self.apply(message)
            }
        }
    }

    func reconcileMany(messages: [Message]) {
        rustBridge.async { [weak self] in
            guard let self else {
                Log.error("CoinControlManager no longer available")
                return
            }

            logger.debug("reconcile_messages: \(messages)")
            DispatchQueue.main.async { [self] in
                for message in messages {
                    self.apply(message)
                }
            }
        }
    }

    public func dispatch(action: Action) { dispatch(action) }
    public func dispatch(_ action: Action) {
        rustBridge.async {
            self.logger.debug("dispatch: \(action)")
            self.rust.dispatch(action: action)
        }
    }
}
