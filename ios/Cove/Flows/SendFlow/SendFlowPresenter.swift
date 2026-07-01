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

    var focusField: SetAmountFocusField?
    var sheetState: TaggedItem<SheetState>? = .none
    var alertState: TaggedItem<SendFlowAlertState>? = .none

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

    var showingAlert: Binding<Bool> {
        Binding(
            get: { self.alertState != nil && !self.disappearing },
            set: { if !$0 { self.alertState = .none }}
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

    var alertTitle: String {
        alertState?.item.localizedTitle ?? ""
    }

    func setDisappearing() {
        self.disappearing = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            self.disappearing = false
        }
    }

    @ViewBuilder
    func alertMessage(alert: TaggedItem<SendFlowAlertState>) -> some View {
        switch alert.item {
        case .error, .general, .unableToLoadFees, .feeTooHigh, .highFeeWarning,
             .unableToReadLockedCoins, .balanceStillLoading:
            Text(alert.item.localizedMessage)
        }
    }

    @ViewBuilder
    func alertButtons(alert: TaggedItem<SendFlowAlertState>) -> some View {
        switch alert.item {
        case let .error(error):
            errorAlertButtons(error)
        case .general, .unableToLoadFees, .feeTooHigh, .highFeeWarning,
             .unableToReadLockedCoins, .balanceStillLoading:
            Button("OK") { self.alertState = .none }
        }
    }

    @ViewBuilder
    private func errorAlertButtons(_ error: SendFlowError) -> some View {
        switch error {
        case .EmptyAddress, .WrongNetwork, .InvalidAddress:
            Button("OK") {
                self.alertState = .none
                self.focusField = .address
            }
        case .NoBalance:
            Button("Go Back") {
                self.alertState = .none
                self.app.popRoute()
            }
        case .InvalidNumber, .InsufficientFunds, .SendAmountToLow, .ZeroAmount, .WalletManager, .UnableToGetFeeDetails:
            Button("OK") {
                self.focusField = .amount
                self.alertState = .none
            }
        case .UnableToGetFeeRate, .UnableToBuildTxn, .UnableToSaveUnsignedTransaction:
            Button("OK") {
                self.focusField = .amount
                self.alertState = .none
            }
        case .UnableToGetMaxSend:
            Button("OK") {
                self.focusField = .amount
                self.alertState = .none
            }
        }
    }
}
