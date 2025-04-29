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

    private var _enteringBtcAmount: String = ""
    private var _enteringFiatAmount: String = ""

    var address: Address? = nil
    var amount: Amount? = nil
    var fiatAmount: Double? = nil

    var presenter: SendFlowPresenter
    var selectedFeeRate: FeeRateOptionWithTotalFee? = nil
    var feeRateOptions: FeeRateOptionsWithTotalFee? = nil
    var maxSelected: Amount? = nil

    var enteringBtcAmount: Binding<String> {
        Binding(
            get: { self._enteringBtcAmount },
            set: { self.dispatch(action: .changeEnteringBtcAmount($0)) }
        )
    }

    var enteringFiatAmount: Binding<String> {
        Binding(
            get: { self._enteringFiatAmount },
            set: { self.dispatch(action: .changeEnteringFiatAmount($0)) }
        )
    }

    public init(_ rust: RustSendFlowManager, presenter: SendFlowPresenter) {
        self.rust = rust
        self.presenter = presenter

        self.rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    func reconcile(message: SendFlowManagerReconcileMessage) {
        Task { [weak self] in
            guard let self else {
                Log.error("SendFlowManager no longer available")
                return
            }

            self.logger.debug("reconcile: \(message)")

            await MainActor.run {
                switch message {
                case let .updateAmountFiat(fiat):
                    self.fiatAmount = fiat
                case let .updateAmountSats(sats):
                    self.amount = Amount.fromSat(sats: sats)
                case let .updateFeeRateOptions(options):
                    self.feeRateOptions = options
                case let .updateEnteringBtcAmount(amount):
                    self._enteringBtcAmount = amount
                case let .updateEnteringFiatAmount(amount):
                    self._enteringFiatAmount = amount
                case let .updateSelectedFeeRate(rate):
                    self.selectedFeeRate = rate
                case let .updateMaxSelected(max):
                    self.maxSelected = max
                case let .updateFeeRate(rate):
                    self.selectedFeeRate = rate
                case let .updateFocusField(field):
                    self.presenter.focusField = field
                case let .setAlert(alertState):
                    self.presenter.alertState = alertState.map(TaggedItem.init)
                }
            }
        }
    }

    public func dispatch(action: SendFlowManagerAction) {
        self.logger.debug("dispatch: \(action)")
        self.rust.dispatch(action: action)
    }
}
