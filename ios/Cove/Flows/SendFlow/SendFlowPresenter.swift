//
//  SendFlowPresenter.swift
//  Cove
//
//  Created by Praveen Perera on 11/20/24.
//

import SwiftUI

@Observable class SendFlowPresenter {
    typealias FocusField = SetAmountFocusField

    @ObservationIgnored
    let app: AppManager

    @ObservationIgnored
    let manager: WalletManager

    private var disappearing: Bool = false
    @ObservationIgnored
    private var disappearingResetWorkItem: DispatchWorkItem?
    var isDisappearing: Bool {
        disappearing
    }

    var focusField: SetAmountFocusField?
    var sheetState: TaggedItem<SheetState>? = .none
    var alertState: TaggedItem<SendFlowAlertState>? = .none
    var confirmationAlertState: TaggedItem<SendFlowConfirmAlertState>? = .none

    var lastWorkingFeeRate: Float?
    var erroredFeeRate: Float?

    init(app: AppManager, manager: WalletManager) {
        self.app = app
        self.manager = manager
    }

    enum SheetState: Equatable {
        case qr
        case fee
    }

    var alertStateBinding: Binding<TaggedItem<SendFlowAlertState>?> {
        Binding(
            get: {
                if self.disappearing { return nil }
                return self.alertState
            },
            set: { newValue in
                if newValue == nil {
                    self.clearAlert()
                } else {
                    self.alertState = newValue
                }
            }
        )
    }

    var confirmationAlertStateBinding: Binding<TaggedItem<SendFlowConfirmAlertState>?> {
        Binding(
            get: { self.confirmationAlertState },
            set: { self.confirmationAlertState = $0 }
        )
    }

    var sheetStateBinding: Binding<TaggedItem<SheetState>?> {
        Binding(
            get: { self.sheetState },
            set: { newValue in
                self.sheetState = newValue
            }
        )
    }

    func setDisappearing() {
        disappearingResetWorkItem?.cancel()
        self.disappearing = true

        let workItem = DispatchWorkItem { [weak self] in
            self?.disappearing = false
        }
        disappearingResetWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: workItem)
    }

    func clearAlert() {
        if alertState != nil {
            setDisappearing()
        }

        alertState = .none
    }
}
