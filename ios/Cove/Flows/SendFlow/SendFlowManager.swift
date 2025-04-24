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

    var btcAmount: Amount = .fromSat(sats: 0)
    var fiatAmount: Double = 0.0

    var selectedFeeRate: FeeRateOptionWithTotalFee? = nil
    var feeRateOptions: FeeRateOptionsWithTotalFee? = nil

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

    public init(_ rust: RustSendFlowManager) {
        self.rust = rust
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
                        self.btcAmount = Amount.fromSat(sats: sats)
                }
            }
        }
    }

    public func dispatch(action: SendFlowManagerAction) {
        self.logger.debug("dispatch: \(action)")
        self.rust.dispatch(action: action)
    }
}
