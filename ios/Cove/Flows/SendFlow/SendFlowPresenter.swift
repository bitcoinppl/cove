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

    var disappearing: Bool = false

    var focusField: SetAmountFocusField?
    var sheetState: TaggedItem<SheetState>? = .none
    var alertState: TaggedItem<SendFlowAlertState>? = .none

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
            set: { newValue in
                if !newValue {
                    self.alertState = .none
                }
            }
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
        switch alertState?.item {
        case let .error(error):
            errorAlertTitle(error)
        case .none:
            ""
        }
    }

    private func errorAlertTitle(_ error: SendFlowError) -> String {
        switch error {
        case .EmptyAddress, .InvalidAddress, .WrongNetwork:
            "Invalid Address"
        case .InvalidNumber, .ZeroAmount: "Invalid Amount"
        case .InsufficientFunds, .NoBalance: "Insufficient Funds"
        case .SendAmountToLow: "Send Amount Too Low"
        case .UnableToGetFeeRate: "Unable to get fee rate"
        case .UnableToBuildTxn: "Unable to build transaction"
        case .UnableToGetMaxSend:
            "Unable to get max send"
        case .UnableToSaveUnsignedTransaction:
            "Unable to Save Unsigned Transaction"
        case .WalletManagerError:
            "Error"
        case .UnableToGetFeeDetails:
            "Fee Details Error"
        }
    }

    @ViewBuilder
    func alertMessage(alert: TaggedItem<SendFlowAlertState>) -> some View {
        switch alert.item {
        case let .error(error):
            Text(errorAlertMessage(error))
        }
    }

    private func errorAlertMessage(_ error: SendFlowError) -> String {
        switch error {
        case .EmptyAddress:
            "Please enter an address"
        case .InvalidNumber:
            "Please enter a valid number for the amout to send"
        case .ZeroAmount:
            "Can't send an empty transaction. Please enter a valid amount"
        case .NoBalance:
            "You do not have any bitcoin in your wallet. Please add some to send a transaction"
        case let .InvalidAddress(address):
            "The address \(address) is invalid"
        case let .WrongNetwork(address: address, validFor: validFor, current: currentNetwork):
            "The address \(address) is on the wrong network, is it for (\(validFor). You are on \(currentNetwork)"
        case .InsufficientFunds:
            "You do not have enough bitcoin in your wallet to cover the amount plus fees"
        case .SendAmountToLow:
            "Send amount is too low. Please send atleast 5000 sats"
        case .UnableToGetFeeRate:
            "Are you connected to the internet?"
        case let .WalletManagerError(msg):
            msg.describe
        case let .UnableToGetFeeDetails(msg):
            msg
        case let .UnableToBuildTxn(msg):
            msg
        case let .UnableToGetMaxSend(msg):
            msg
        case let .UnableToSaveUnsignedTransaction(msg):
            msg
        }
    }

    @ViewBuilder
    func alertButtons(alert: TaggedItem<SendFlowAlertState>) -> some View {
        switch alert.item {
        case let .error(error):
            errorAlertButtons(error)
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
        case .InvalidNumber, .InsufficientFunds, .SendAmountToLow, .ZeroAmount, .WalletManagerError, .UnableToGetFeeDetails:
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
