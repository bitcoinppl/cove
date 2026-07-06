import SwiftUI

struct SendFlowHardwarePresentationContext {
    let manager: WalletManager
    let details: ConfirmDetails
    let alertState: Binding<TaggedItem<SendFlowHardwareAlertState>?>
    let inputOutputDetailsPresentationSize: Binding<PresentationDetent>

    func dismissAlert() {
        alertState.wrappedValue = nil
    }
}

extension SendFlowHardwareAlertState: TaggedAlertPresentable {
    func alert(context: SendFlowHardwarePresentationContext) -> AnyAlertBuilder {
        let singleOkCancel = {
            Button("Ok", role: .cancel) {
                context.dismissAlert()
            }
        }

        switch self {
        case let .bbqrError(message):
            return AlertBuilder(
                title: "QR Error",
                message: "Unable to create BBQr: \(message)",
                actions: singleOkCancel
            ).eraseToAny()

        case let .fileError(message):
            return AlertBuilder(
                title: "File Import Error",
                message: message,
                actions: singleOkCancel
            ).eraseToAny()

        case let .nfcError(error):
            return AlertBuilder(
                title: "NFC Error",
                message: error,
                actions: singleOkCancel
            ).eraseToAny()

        case let .pasteError(error):
            return AlertBuilder(
                title: "Paste Error",
                message: error,
                actions: singleOkCancel
            ).eraseToAny()
        }
    }
}
