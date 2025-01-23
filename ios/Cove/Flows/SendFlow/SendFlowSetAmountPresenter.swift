//
//  SendFlowSetAmountPresenter.swift
//  Cove
//
//  Created by Praveen Perera on 11/20/24.
//

import SwiftUI

@Observable class SendFlowSetAmountPresenter {
    @ObservationIgnored
    let app: AppManager

    @ObservationIgnored
    let manager: WalletManager

    var amount: Amount?
    var address: Address?
    var maxSelected: Amount?

    var disappearing: Bool = false
    var focusField: FocusField?
    var sheetState: TaggedItem<SheetState>? = .none
    var alertState: TaggedItem<AlertState>? = .none

    init(app: AppManager, manager: WalletManager) {
        self.app = app
        self.manager = manager
    }

    enum FocusField: Hashable {
        case amount
        case address
    }

    enum SheetState: Equatable {
        case qr
        case fee
    }

    enum AlertState: Equatable {
        case emptyAddress
        case invalidNumber
        case invalidAddress(String)
        case wrongNetwork(String)
        case noBalance
        case zeroAmount
        case insufficientFunds
        case sendAmountToLow
        case unableToGetFeeRate
        case unableToBuildTxn(String)

        init(_ error: AddressError, address: String) {
            switch error {
            case .EmptyAddress: self = .emptyAddress
            case .InvalidAddress: self = .invalidAddress(address)
            case .WrongNetwork: self = .wrongNetwork(address)
            default: self = .invalidAddress(address)
            }
        }
    }

    func setAlertState(_ alertState: AlertState) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            guard !self.disappearing else { return }
            self.alertState = TaggedItem(alertState)
        }
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
        guard let alertState else { return "" }

        return {
            switch alertState.item {
            case .emptyAddress, .invalidAddress, .wrongNetwork:
                "Invalid Address"
            case .invalidNumber, .zeroAmount: "Invalid Amount"
            case .insufficientFunds, .noBalance: "Insufficient Funds"
            case .sendAmountToLow: "Send Amount Too Low"
            case .unableToGetFeeRate: "Unable to get fee rate"
            case .unableToBuildTxn: "Unable to build transaction"
            }
        }()
    }

    @ViewBuilder
    func alertMessage(alert: TaggedItem<AlertState>) -> some View {
        let text =
            switch alert.item {
            case .emptyAddress:
                "Please enter an address"
            case .invalidNumber:
                "Please enter a valid number for the amout to send"
            case .zeroAmount:
                "Can't send an empty transaction. Please enter a valid amount"
            case .noBalance:
                "You do not have any bitcoin in your wallet. Please add some to send a transaction"
            case let .invalidAddress(address):
                "The address \(address) is invalid"
            case let .wrongNetwork(address):
                "The address \(address) is on the wrong network. You are on \(manager.walletMetadata.network)"
            case .insufficientFunds:
                "You do not have enough bitcoin in your wallet to cover the amount plus fees"
            case .sendAmountToLow:
                "Send amount is too low. Please send atleast 5000 sats"
            case .unableToGetFeeRate:
                "Are you connected to the internet?"
            case let .unableToBuildTxn(msg):
                msg
            }

        Text(text)
    }

    @ViewBuilder
    func alertButtons(alert: TaggedItem<AlertState>) -> some View {
        switch alert.item {
        case .emptyAddress, .wrongNetwork, .invalidAddress:
            Button("OK") {
                self.alertState = .none
                self.focusField = .address
            }
        case .noBalance:
            Button("Go Back") {
                self.alertState = .none
                self.app.popRoute()
            }
        case .invalidNumber, .insufficientFunds, .sendAmountToLow, .zeroAmount:
            Button("OK") {
                self.focusField = .amount
                self.alertState = .none
            }
        case .unableToGetFeeRate, .unableToBuildTxn:
            Button("OK") {
                self.focusField = .none
                self.alertState = .none
            }
        }
    }
}
