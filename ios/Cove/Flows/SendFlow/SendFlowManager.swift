//
//  SendFlowManager.swift
//  Cove
//
//  Created by Praveen Perera on 4/24/25.
//

import Foundation
import os
import SwiftUI

extension WeakReconciler: SendFlowManagerReconciler where Reconciler == SendFlowManager {}

protocol SendFlowRustManaging: AnyObject, Sendable {
    func walletId() -> WalletId
    func enteringFiatAmount() -> String
    func sendAmountFiat() -> String
    func sendAmountBtc() -> String
    func totalSpentInFiat() -> String
    func totalSpentInBtc() -> String
    func totalFeeString() -> String?
    func listenForUpdates(reconciler: SendFlowManagerReconciler)
    func validateAddress(displayAlert: Bool) -> Bool
    func validateAmount(displayAlert: Bool) -> Bool
    func getCustomFeeOption(
        feeRate: FeeRate,
        feeSpeed: FeeSpeed
    ) async throws -> FeeRateOptionWithTotalFee
    func waitForInit() async -> Bool
    func amountExceedsBalance() -> Bool
    func sanitizeBtcEnteringAmount(oldValue: String, newValue: String) -> String?
    func sanitizeFiatEnteringAmount(oldValue: String, newValue: String) -> String?
    func utxos() -> [Utxo]?
    func maxSendMinusFees() -> Amount?
    func maxSendMinusFeesAndSmallUtxo() -> Amount?
    func dispatch(action: SendFlowManagerAction)
}

extension RustSendFlowManager: SendFlowRustManaging {}

private enum SendFlowManagerAccessError: LocalizedError {
    case closed

    var errorDescription: String? {
        "Send flow manager is closed"
    }
}

@Observable final class SendFlowManager: ReconcilingManager, SendFlowManagerReconciler {
    typealias Message = SendFlowManagerReconcileMessage
    typealias Action = SendFlowManagerAction

    private struct RustState {
        var rust: SendFlowRustManaging? = nil
        var debouncedTask: Task<Void, Never>? = nil
        var delayedAlertWorkItem: DispatchWorkItem? = nil
        var isClosed = false
    }

    private let logger = Log(id: "SendFlowManager")
    @ObservationIgnored
    private let rustState = OSAllocatedUnfairLock(initialState: RustState())
    @ObservationIgnored
    private let rustBridge = DispatchQueue(
        label: "cove.SendFlowManager.rustbridge", qos: .userInitiated
    )

    @ObservationIgnored
    let id: WalletId

    private var rust: SendFlowRustManaging? {
        rustState.withLock { $0.rust }
    }

    var enteringBtcAmount: String = ""
    var enteringFiatAmount: String = ""
    private var _enteringAddress: String = ""

    var address: Address? = nil
    var amount: Amount? = nil
    var fiatAmount: Double? = nil

    var presenter: SendFlowPresenter
    var feeSelection: FeeSelection? = nil
    var selectedFeeRate: FeeRateOptionWithTotalFee? {
        feeSelection?.selected
    }

    var feeRateOptions: FeeRateOptionsWithTotalFee? {
        feeSelection?.options
    }

    var maxSelected: Amount? = nil

    // presenting
    var sendAmountFiat: String = ""
    var sendAmountBtc: String = ""

    var totalSpentInFiat: String = ""
    var totalSpentInBtc: String = ""
    var totalFeeString: String? = nil

    var enteringAddress: Binding<String> {
        Binding<String>(
            get: { self._enteringAddress },
            set: { newValue in
                guard self.canApplyReconcileMessages else { return }

                self._enteringAddress = newValue
                self.dispatch(action: .notifyEnteringAddressChanged(newValue))
            }
        )
    }

    func validate(displayAlert: Bool = false) -> Bool {
        validateAmount(displayAlert: displayAlert)
            && validateAddress(displayAlert: displayAlert)
    }

    func validateAddress(displayAlert: Bool = false) -> Bool {
        withRustOr(false) { $0.validateAddress(displayAlert: displayAlert) }
    }

    func validateAmount(displayAlert: Bool = false) -> Bool {
        withRustOr(false) { $0.validateAmount(displayAlert: displayAlert) }
    }

    init(_ rust: SendFlowRustManaging, presenter: SendFlowPresenter) {
        self.presenter = presenter

        self.id = rust.walletId()
        self.enteringFiatAmount = rust.enteringFiatAmount()
        self.sendAmountFiat = rust.sendAmountFiat()
        self.sendAmountBtc = rust.sendAmountBtc()
        self.totalSpentInFiat = rust.totalSpentInFiat()
        self.totalSpentInBtc = rust.totalSpentInBtc()
        self.totalFeeString = rust.totalFeeString()

        rustState.withLock { $0.rust = rust }
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    deinit {
        close()
    }

    func close() {
        let state = takeRustAndQueuedWorkForClose()
        guard state.rust != nil else { return }

        state.debouncedTask?.cancel()
        state.delayedAlertWorkItem?.cancel()
        logger.debug("Closed SendFlowManager for wallet \(id)")
    }

    private func takeRustAndQueuedWorkForClose() -> (
        rust: SendFlowRustManaging?,
        debouncedTask: Task<Void, Never>?,
        delayedAlertWorkItem: DispatchWorkItem?
    ) {
        rustState.withLock { state in
            guard !state.isClosed else { return (nil, nil, nil) }

            state.isClosed = true
            let rust = state.rust
            let debouncedTask = state.debouncedTask
            let delayedAlertWorkItem = state.delayedAlertWorkItem
            state.rust = nil
            state.debouncedTask = nil
            state.delayedAlertWorkItem = nil
            return (rust, debouncedTask, delayedAlertWorkItem)
        }
    }

    private func requireRust() throws -> SendFlowRustManaging {
        guard let rust else { throw SendFlowManagerAccessError.closed }
        return rust
    }

    private func withRustOr<T>(
        _ defaultValue: T,
        _ body: (SendFlowRustManaging) -> T
    ) -> T {
        guard let rust else { return defaultValue }
        return body(rust)
    }

    private func installDebouncedTask(_ task: Task<Void, Never>) {
        let previousTask = rustState.withLock { state -> Task<Void, Never>? in
            guard !state.isClosed else {
                task.cancel()
                return nil
            }

            let previousTask = state.debouncedTask
            state.debouncedTask = task
            return previousTask
        }

        previousTask?.cancel()
    }

    private func installDelayedAlertWorkItem(_ workItem: DispatchWorkItem?) {
        let previousWorkItem = rustState.withLock { state -> DispatchWorkItem? in
            guard !state.isClosed else {
                workItem?.cancel()
                return nil
            }

            let previousWorkItem = state.delayedAlertWorkItem
            state.delayedAlertWorkItem = workItem
            return previousWorkItem
        }

        previousWorkItem?.cancel()
    }

    public func setAddress(_ address: Address) {
        guard canApplyReconcileMessages else { return }

        self._enteringAddress = address.unformatted()
        self.address = address
        self.dispatch(action: .notifyAddressChanged(address))
    }

    public func setAmount(_ amount: Amount) {
        guard canApplyReconcileMessages else { return }

        self.amount = amount
        self.dispatch(action: .notifyAmountChanged(amount))
    }

    public func refreshPresenters() {
        guard let rust else { return }

        self.totalSpentInFiat = rust.totalSpentInFiat()
        self.totalSpentInBtc = rust.totalSpentInBtc()
        self.totalFeeString = rust.totalFeeString()
        self.sendAmountBtc = rust.sendAmountBtc()
        self.sendAmountFiat = rust.sendAmountFiat()
    }

    public func reconcileAfterLabelsChanged() {
        dispatch(action: .refreshWalletBalance)
    }

    public func getNewCustomFeeRateWithTotal(
        feeRate: FeeRate, feeSpeed: FeeSpeed
    ) async throws -> FeeRateOptionWithTotalFee {
        let rust = try requireRust()
        return try await rust.getCustomFeeOption(
            feeRate: feeRate, feeSpeed: feeSpeed
        )
    }

    func waitForInit() async -> Bool {
        guard let rust else { return false }
        return await rust.waitForInit()
    }

    func amountExceedsBalance() -> Bool {
        withRustOr(false) { $0.amountExceedsBalance() }
    }

    func sanitizeBtcEnteringAmount(oldValue: String, newValue: String) -> String? {
        withRustOr(nil) {
            $0.sanitizeBtcEnteringAmount(oldValue: oldValue, newValue: newValue)
        }
    }

    func sanitizeFiatEnteringAmount(oldValue: String, newValue: String) -> String? {
        withRustOr(nil) {
            $0.sanitizeFiatEnteringAmount(oldValue: oldValue, newValue: newValue)
        }
    }

    func utxos() -> [Utxo]? {
        withRustOr(nil) { $0.utxos() }
    }

    func maxSendMinusFees() -> Amount? {
        withRustOr(nil) { $0.maxSendMinusFees() }
    }

    func maxSendMinusFeesAndSmallUtxo() -> Amount? {
        withRustOr(nil) { $0.maxSendMinusFeesAndSmallUtxo() }
    }

    var canApplyReconcileMessages: Bool {
        rust != nil
    }

    func apply(_ message: Message) {
        guard canApplyReconcileMessages else { return }

        switch message {
        case let .updateAmountFiat(fiat):
            self.fiatAmount = fiat

        case let .updateAmountSats(sats):
            self.refreshPresenters()
            self.amount = Amount.fromSat(sats: sats)

        case let .updateFeeSelection(selection):
            self.refreshPresenters()
            self.feeSelection = selection

        case let .updateAddress(address):
            self.address = address

        case let .updateEnteringBtcAmount(amount):
            self.enteringBtcAmount = amount

        case let .updateEnteringAddress(address):
            self._enteringAddress = address

        case let .updateEnteringFiatAmount(amount):
            self.enteringFiatAmount = amount

        case let .updateFocusField(field):
            self.presenter.focusField = field

        case let .setAlert(alertState):
            applySetAlert(alertState)

        case .clearAlert:
            installDelayedAlertWorkItem(nil)
            self.presenter.clearAlert()

        case let .setMaxSelected(maxSelected):
            self.maxSelected = maxSelected

        case .unsetMaxSelected:
            self.maxSelected = nil

        case .refreshPresenters:
            self.refreshPresenters()
        }
    }

    private func applySetAlert(_ alertState: SendFlowAlertState) {
        Log.warn("setAlert: \(alertState)")
        let hadSheet = presenter.sheetState != .none
        let hadAlert = presenter.alertState != .none
        let isDismissingAlert = presenter.isDisappearing

        if hadSheet || hadAlert || isDismissingAlert {
            presenter.clearAlert()
            presenter.sheetState = .none

            let workItem = DispatchWorkItem { [weak self] in
                guard let self, self.canApplyReconcileMessages else { return }

                self.presenter.alertState = .init(alertState)
            }
            installDelayedAlertWorkItem(workItem)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6, execute: workItem)
        } else {
            installDelayedAlertWorkItem(nil)
            presenter.alertState = .init(alertState)
        }
    }

    func logReconcile(message: Message) {
        logger.debug("reconcile: \(message)")
    }

    func logReconcileMany(messages: [Message]) {
        logger.debug("reconcile_messages: \(messages)")
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

    public func debouncedDispatch(
        _ action: Action, for debounceDelay: Duration? = .milliseconds(66)
    ) {
        guard let debounceDelay else { return self.dispatch(action) }

        let task = Task { [weak self] in
            guard let self else { return }
            try? await Task.sleep(for: debounceDelay)
            guard !Task.isCancelled else { return }
            self.dispatch(action)
        }
        installDebouncedTask(task)
    }
}
