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
    private var _enteringAddress: String = ""

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
            set: { newValue in
                guard self._enteringBtcAmount != newValue else { return }
                self.dispatch(action: .changeEnteringBtcAmount(newValue))
            }
        )
    }

    var enteringFiatAmount: Binding<String> {
        binding(\._enteringFiatAmount) { .changeEnteringFiatAmount($0) }
    }

    var enteringAddress: Binding<String> {
        binding(\._enteringAddress) { .changeEnteringAddress($0) }
    }

    private func binding(
        _ keyPath: KeyPath<SendFlowManager, String>,
        _ action: @escaping (String) -> SendFlowManagerAction
    ) -> Binding<String> {
        Binding<String>(
            get: { self[keyPath: keyPath] },
            set: { newValue in
                self.dispatch(action: action(newValue))
            }
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

            logger.debug("reconcile: \(message)")

            await MainActor.run {
                withAnimation {
                    switch message {
                        case let .updateAmountFiat(fiat):
                            self.fiatAmount = fiat
                        case let .updateAmountSats(sats):
                            self.amount = Amount.fromSat(sats: sats)
                        case let .updateFeeRateOptions(options):
                            self.feeRateOptions = options
                        case let .updateEnteringBtcAmount(amount):
                            self._enteringBtcAmount = amount
                        case let .updateEnteringAddress(address):
                            self._enteringAddress = address
                        case let .updateEnteringFiatAmount(amount):
                            self._enteringFiatAmount = amount
                        case let .updateSelectedFeeRate(rate):
                            self.selectedFeeRate = rate
                        case let .updateFeeRate(rate):
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
                    }
                }
            }
        }
    }

    public func dispatch(action: SendFlowManagerAction) {
        logger.debug("dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
