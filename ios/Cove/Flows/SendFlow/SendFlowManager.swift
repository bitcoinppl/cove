//
//  SendFlowManager.swift
//  Cove
//
//  Created by Praveen Perera on 4/24/25.
//

import Foundation
import SwiftUI

extension WeakReconciler: SendFlowManagerReconciler where Reconciler == SendFlowManager {}

@Observable final class SendFlowManager: AnyReconciler, SendFlowManagerReconciler {
    private let logger = Log(id: "SendFlowManager")
    var rust: RustSendFlowManager

    var enteringBtcAmount: String = ""
    var enteringFiatAmount: String = ""
    private var _enteringAddress: String = ""

    var address: Address? = nil
    var amount: Amount? = nil
    var fiatAmount: Double? = nil

    var presenter: SendFlowPresenter
    var selectedFeeRate: FeeRateOptionWithTotalFee? = nil
    var feeRateOptions: FeeRateOptionsWithTotalFee? = nil
    var maxSelected: Amount? = nil

    // presenting
    var sendAmountFiat: String = ""
    var sendAmountBtc: String = ""

    var totalSpentInFiat: String = ""
    var totalSpentInBtc: String = ""
    var totalFeeString: String = ""

    var enteringAddress: Binding<String> {
        Binding<String>(
            get: { self._enteringAddress },
            set: { newValue in
                self._enteringAddress = newValue
                self.dispatch(action: .notifyEnteringAddressChanged(newValue))
            }
        )
    }

    public init(_ rust: RustSendFlowManager, presenter: SendFlowPresenter) {
        self.rust = rust
        self.presenter = presenter

        self.enteringFiatAmount = rust.enteringFiatAmount()
        self.sendAmountFiat = rust.sendAmountFiat()
        self.sendAmountBtc = rust.sendAmountBtc()
        self.totalSpentInFiat = rust.totalSpentInFiat()
        self.totalSpentInBtc = rust.totalSpentInBtc()
        self.totalFeeString = rust.totalFeeString()

        self.rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    public func setAddress(_ address: Address) {
        self._enteringAddress = address.string()
        self.address = address
        self.dispatch(action: .notifyAddressChanged(address))
    }

    public func setAmount(_ amount: Amount) {
        self.amount = amount
        self.dispatch(action: .notifyAmountChanged(amount))
    }

    public func refreshPresenters() {
        self.totalSpentInFiat = self.rust.totalSpentInFiat()
        self.totalSpentInBtc = self.rust.totalSpentInBtc()
        self.totalFeeString = self.rust.totalFeeString()
        self.sendAmountBtc = self.rust.sendAmountBtc()
        self.sendAmountFiat = self.rust.sendAmountFiat()
    }

    func reconcile(message: SendFlowManagerReconcileMessage) {
        Task { [weak self] in
            guard let self else {
                Log.error("SendFlowManager no longer available")
                return
            }

            logger.debug("reconcile: \(message)")
            await MainActor.run {
                switch message {
                case let .updateAmountFiat(fiat):
                    self.fiatAmount = fiat

                case let .updateAmountSats(sats):
                    self.refreshPresenters()
                    self.amount = Amount.fromSat(sats: sats)

                case let .updateFeeRateOptions(options):
                    self.refreshPresenters()
                    self.feeRateOptions = options

                case let .updateAddress(address):
                    self.address = address

                case let .updateEnteringBtcAmount(amount):
                    self.enteringBtcAmount = amount

                case let .updateEnteringAddress(address):
                    self._enteringAddress = address

                case let .updateEnteringFiatAmount(amount):
                    self.enteringFiatAmount = amount

                case let .updateSelectedFeeRate(rate):
                    self.refreshPresenters()
                    self.selectedFeeRate = rate

                case let .updateFocusField(field):
                    self.presenter.focusField = field

                case let .setAlert(alertState):
                    self.presenter.alertState = .init(alertState)

                case .clearAlert:
                    self.presenter.alertState = .none

                case let .setMaxSelected(maxSelected):
                    self.maxSelected = maxSelected

                case .unsetMaxSelected:
                    self.maxSelected = nil

                case .refreshPresenters:
                    self.refreshPresenters()
                }
            }
        }
    }

    private let dispatchQueue = DispatchQueue(label: "cove.sendflowmanager.dispatch", qos: .userInitiated)
    public func dispatch(action: SendFlowManagerAction) { dispatch(action) }
    public func dispatch(_ action: SendFlowManagerAction) {
        dispatchQueue.async {
            self.logger.debug("dispatch: \(action)")
            self.rust.dispatch(action: action)
        }
    }
}
