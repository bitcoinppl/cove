import SwiftUI

enum SendFlowConfirmAlertState: Equatable {
    case sent(WalletId)
    case broadcastError(String)
}

struct SendFlowErrorAlertContext {
    let alertState: Binding<TaggedItem<SendFlowErrorAlert>?>

    func dismissAlert() {
        alertState.wrappedValue = nil
    }
}

struct SendFlowConfirmAlertContext {
    let presenter: SendFlowPresenter
    let sendState: Binding<SendState>

    func dismissAlert() {
        presenter.confirmationAlertState = nil
    }
}

struct SendFlowAlertContext {
    let presenter: SendFlowPresenter
    let sendFlowManager: SendFlowManager

    func dismissAlert() {
        presenter.clearAlert()
    }
}

extension SendFlowAlertState: TaggedAlertPresentable {
    func alert(context: SendFlowAlertContext) -> AnyAlertBuilder {
        AlertBuilder(
            title: title,
            message: { Text(message) },
            actions: { SendFlowAlertActions(alert: self, context: context) }
        ).eraseToAny()
    }
}

extension SendFlowErrorAlert: TaggedAlertPresentable {
    func alert(context: SendFlowErrorAlertContext) -> AnyAlertBuilder {
        AlertBuilder(
            title: "Error!",
            message: message,
            actions: {
                Button("OK") {
                    context.dismissAlert()
                }
            }
        ).eraseToAny()
    }
}

extension SendFlowConfirmAlertState: TaggedAlertPresentable {
    func alert(context: SendFlowConfirmAlertContext) -> AnyAlertBuilder {
        switch self {
        case let .sent(walletId):
            AlertBuilder(
                title: "Sent!",
                message: "Transaction was successfully sent!",
                actions: {
                    Button("OK") {
                        context.dismissAlert()
                        context.presenter.app.loadAndReset(to: Route.selectedWallet(walletId))
                    }
                }
            ).eraseToAny()

        case let .broadcastError(error):
            AlertBuilder(
                title: "Error Broadcasting!",
                message: error,
                actions: {
                    Button("OK") {
                        context.sendState.wrappedValue = .idle
                        context.dismissAlert()
                    }
                }
            ).eraseToAny()
        }
    }
}

private struct SendFlowAlertActions: View {
    let alert: SendFlowAlertState
    let context: SendFlowAlertContext

    private var presenter: SendFlowPresenter {
        context.presenter
    }

    var body: some View {
        switch alert {
        case let .error(error):
            errorButtons(error)
        case .general:
            Button("OK") { context.dismissAlert() }
        case let .warning(kind: kind, title: _, message: _):
            Button("Send Anyway") {
                context.sendFlowManager.dispatch(action: .acknowledgeWarningAndFinalize(kind))
            }
            Button("Cancel", role: .cancel) {
                context.dismissAlert()
            }
        }
    }

    @ViewBuilder
    private func errorButtons(_ error: SendFlowError) -> some View {
        switch error {
        case .EmptyAddress, .WrongNetwork, .InvalidAddress:
            Button("OK") {
                context.dismissAlert()
                presenter.focusField = .address
            }

        case .NoBalance:
            Button("Go Back") {
                context.dismissAlert()
                presenter.app.popRoute()
            }

        case .InvalidNumber,
             .InsufficientFunds,
             .SendBelowDustLimit,
             .ZeroAmount,
             .WalletManager,
             .UnableToGetFeeDetails,
             .UnableToGetFeeRate,
             .UnableToBuildTxn,
             .UnableToSaveUnsignedTransaction,
             .UnableToGetMaxSend:
            Button("OK") {
                presenter.focusField = .amount
                context.dismissAlert()
            }
        }
    }
}

private extension SendFlowAlertState {
    var title: String {
        switch self {
        case let .error(error):
            error.title
        case let .general(title: title, message: _):
            title
        case let .warning(kind: _, title: title, message: _):
            title
        }
    }

    var message: String {
        switch self {
        case let .error(error):
            error.message
        case let .general(title: _, message: message):
            message
        case let .warning(kind: _, title: _, message: message):
            message
        }
    }
}

private extension SendFlowErrorAlert {
    var message: String {
        switch self {
        case let .confirmDetails(error):
            error
        case let .signAndBroadcast(error):
            error
        }
    }
}

private extension SendFlowError {
    var title: String {
        switch self {
        case .EmptyAddress, .InvalidAddress, .WrongNetwork:
            "Invalid Address"
        case .InvalidNumber, .ZeroAmount:
            "Invalid Amount"
        case .InsufficientFunds, .NoBalance:
            "Insufficient Funds"
        case .SendBelowDustLimit:
            "Amount Below Dust Limit"
        case .UnableToGetFeeRate:
            "Unable to get fee rate"
        case .UnableToBuildTxn:
            "Unable to build transaction"
        case .UnableToGetMaxSend:
            "Unable to get max send"
        case .UnableToSaveUnsignedTransaction:
            "Unable to Save Unsigned Transaction"
        case .WalletManager(.LockedOutputsSelected):
            "Insufficient Funds"
        case .WalletManager:
            "Error"
        case .UnableToGetFeeDetails:
            "Fee Details Error"
        }
    }

    var message: String {
        switch self {
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
        case .SendBelowDustLimit:
            "This amount is below Bitcoin's dust limit for this address type. The network would reject it. Please send a bit more."
        case .UnableToGetFeeRate:
            "Are you connected to the internet?"
        case .WalletManager(.LockedOutputsSelected):
            "Selected coins include locked coins. Unlock them or choose different coins."
        case let .WalletManager(msg):
            msg.description
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
}
