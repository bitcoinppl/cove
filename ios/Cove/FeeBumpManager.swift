import os
import SwiftUI

extension WeakReconciler: FeeBumpManagerReconciler where Reconciler == FeeBumpManager {}

/// Speeds up one pending send by replacing it with a transaction that pays a higher fee
///
/// The sheet or popup that opens it owns it, and it is never cached: Rust builds the options
/// when it is made, so a new screen gets fresh rates
@Observable final class FeeBumpManager: ReconcilingManager, FeeBumpManagerReconciler {
    typealias Message = FeeBumpManagerReconcileMessage
    typealias Action = FeeBumpManagerAction

    private let logger = Log(id: "FeeBumpManager")

    @ObservationIgnored
    private let rust: RustFeeBumpManager

    @ObservationIgnored
    private let rustBridge = DispatchQueue(label: "cove.FeeBumpManager.rustbridge", qos: .userInitiated)

    let txId: TxId

    private(set) var status: FeeBumpStatus
    private(set) var selected: FeeRateOptionWithTotalFee?

    /// Set from Continue until Rust hands over the next screen or says why it can't
    private(set) var isContinuing = false

    /// The confirm or export route Continue built, which the screen that owns this manager pushes
    /// once it has closed
    private(set) var readyRoute: Route?

    var alert: TaggedItem<FeeBumpAlert>?

    /// Counts selections, so a custom rate that finishes pricing late can't replace a later choice
    @ObservationIgnored
    private var selectionCount = 0

    init(_ rust: RustFeeBumpManager, txId: TxId) {
        self.rust = rust
        self.txId = txId
        self.status = rust.status()
        self.selected = rust.selectedFeeRate()

        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    /// The options while they are ready, `nil` while loading or when the send can't be sped up
    var options: FeeBumpOptions? {
        if case let .ready(options) = status { return options }
        return nil
    }

    var unavailableMessage: String? {
        if case let .unavailable(unavailable) = status { return unavailable.userMessage() }
        return nil
    }

    func apply(_ message: Message) {
        switch message {
        case let .updateStatus(status):
            withAnimation { self.status = status }

            // a send that confirmed or was replaced meanwhile can't continue any more
            if case .unavailable = status { isContinuing = false }

        case let .updateSelectedFeeRate(selected):
            withAnimation { self.selected = selected }

        case let .continueFailed(message):
            isContinuing = false
            alert = TaggedItem(.continueFailed(message))

        case let .feeWarning(warning):
            isContinuing = false
            alert = TaggedItem(.feeWarning(warning))

        // stays continuing while the screen closes, so Continue can't be tapped again
        case let .readyToConfirm(route):
            readyRoute = route
        }
    }

    func logReconcile(message: Message) {
        logger.debug("reconcile: \(String(describing: message).prefix { $0 != "(" })")
    }

    func logReconcileMany(messages: [Message]) {
        logger.debug("reconcile_messages: \(messages.count) messages")
    }

    /// The replacement's total fee at a custom rate, failing below the lowest rate a node takes
    func customFeeOption(_ feeRate: FeeRate) async throws -> FeeRateOptionWithTotalFee {
        try await rust.customFeeOption(feeRate: feeRate)
    }

    /// Rust's message for a custom rate that can't pay for the replacement
    static func customRateFailureMessage(_ error: Error) -> String? {
        guard case let .Unavailable(message)? = error as? FeeBumpRateError else { return nil }
        return message
    }

    func select(_ option: FeeRateOptionWithTotalFee) {
        selectionCount += 1
        selected = option
        dispatch(.selectFeeRate(option))
    }

    /// Prices a custom rate and selects it once priced, unless the user picked another fee meanwhile
    ///
    /// For a custom rate sheet that closed before its typed rate finished pricing: the work runs
    /// here, as this manager outlives that sheet
    func selectCustomFeeRate(_ feeRate: FeeRate) {
        let countAtStart = selectionCount

        Task { @MainActor in
            do {
                let option = try await customFeeOption(feeRate)
                guard selectionCount == countAtStart else { return }

                select(option)
            } catch {
                logger.warn("Unable to select a fee bump at \(feeRate.satPerVb()) sat/vB: \(error)")

                // the sheet that would have shown why closed already, so say it here
                guard selectionCount == countAtStart, let message = Self.customRateFailureMessage(error) else { return }
                alert = TaggedItem(.customRateFailed(message))
            }
        }
    }

    /// Builds the replacement and moves on to confirm or export it
    func continueWithSelected() {
        guard selected != nil, options != nil, !isContinuing else { return }

        isContinuing = true
        dispatch(.continue)
    }

    func acknowledge(_ warning: FeeBumpFeeWarning) {
        isContinuing = true
        dispatch(.acknowledgeFeeWarning(warning))
    }

    func screenAppeared() {
        isContinuing = false
        dispatch(.screenAppeared)
    }

    func dispatch(_ action: Action) {
        rustBridge.async { [rust, logger] in
            logger.debug("dispatch: \(String(describing: action).prefix { $0 != "(" })")
            rust.dispatch(action: action)
        }
    }
}

/// What the bump fee screens tell the user in an alert
enum FeeBumpAlert {
    case continueFailed(String)
    case customRateFailed(String)
    case feeWarning(FeeBumpFeeWarning)
}
