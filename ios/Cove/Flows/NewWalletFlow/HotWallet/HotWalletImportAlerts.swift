import SwiftUI

struct HotWalletImportPresentationContext {
    let app: AppManager
    let alertState: Binding<TaggedItem<HotWalletImportAlertState>?>
    let handleScan: (Result<ScanResult, ScanError>) -> Void
    let onImported: ((WalletId) -> Void)?

    func dismissAlert() {
        alertState.wrappedValue = nil
    }
}

extension HotWalletImportAlertState: TaggedAlertPresentable {
    func alert(context: HotWalletImportPresentationContext) -> AnyAlertBuilder {
        let singleOkCancel = {
            Button("Ok", role: .cancel) {
                context.dismissAlert()
            }
        }

        switch self {
        case .invalidWords:
            return AlertBuilder(
                title: "Words not valid",
                message:
                "The words you entered does not create a valid wallet. Please check the words and try again.",
                actions: singleOkCancel
            ).eraseToAny()

        case let .duplicateWallet(walletId):
            return AlertBuilder(
                title: "Duplicate Wallet",
                message: "This wallet has already been imported!",
                actions: {
                    Button("OK", role: .cancel) {
                        context.dismissAlert()
                        if let onImported = context.onImported {
                            onImported(walletId)
                            return
                        }

                        do {
                            try context.app.selectWalletOrThrow(walletId)
                            context.app.resetRoute(to: .selectedWallet(walletId))
                        } catch {
                            context.app.alertState = TaggedItem(.general(
                                title: "Unable to Select Wallet",
                                message: error.localizedDescription
                            ))
                        }
                    }
                }
            ).eraseToAny()

        case let .scanError(error):
            return AlertBuilder(
                title: "Error Scanning QR Code",
                message: error,
                actions: singleOkCancel
            ).eraseToAny()
        }
    }
}
