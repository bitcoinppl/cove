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

        case let .scan(alert):
            return alert.alert(context: context)

        case let .importError(message):
            return AlertBuilder(
                title: "Import Failed",
                message: message,
                actions: singleOkCancel
            ).eraseToAny()
        }
    }
}

extension HotWalletImportScanAlert {
    func alert(context: HotWalletImportPresentationContext) -> AnyAlertBuilder {
        let singleOkCancel = {
            Button("Ok", role: .cancel) {
                context.dismissAlert()
            }
        }

        switch self {
        case .invalidSeedQr:
            return AlertBuilder(
                title: "Invalid QR Code",
                message: "Please scan a valid seed phrase QR code",
                actions: singleOkCancel
            ).eraseToAny()

        case .invalidNfcTag:
            return AlertBuilder(
                title: "Invalid NFC Tag",
                message: "Please scan a valid seed phrase NFC tag",
                actions: singleOkCancel
            ).eraseToAny()

        case let .unsupportedWordCount(wordCount):
            return AlertBuilder(
                title: "Unsupported Recovery Phrase",
                message: "This phrase has \(wordCount) words. Cove supports 12 or 24 word recovery phrases.",
                actions: singleOkCancel
            ).eraseToAny()

        case .noCameraPermission:
            // the same words as the app's own camera alert, which Rust writes
            let appAlert = AppAlertState.noCameraPermission
            return AlertBuilder(
                title: appAlert.title(),
                message: appAlert.message(),
                actions: {
                    Button("Settings") {
                        context.dismissAlert()
                        if let url = URL(string: UIApplication.openSettingsURLString) {
                            UIApplication.shared.open(url)
                        }
                    }

                    Button("Cancel", role: .cancel) {
                        context.dismissAlert()
                    }
                }
            ).eraseToAny()
        }
    }
}
