import SwiftUI

struct SelectedWalletPresentationContext {
    let app: AppManager
    let manager: WalletManager
    let presentationState: Binding<TaggedItem<SelectedWalletPresentationState>?>
    let walletErrorAlert: Binding<TaggedItem<WalletErrorAlert>?>
    let scannedLabels: Binding<TaggedItem<MultiFormat>?>

    func dismissWalletError() {
        walletErrorAlert.wrappedValue = nil
    }
}

extension WalletErrorAlert: TaggedAlertPresentable {
    func alert(context: SelectedWalletPresentationContext) -> AnyAlertBuilder {
        switch self {
        case .nodeConnectionFailed:
            AlertBuilder(
                title: "Node Connection Failed",
                message: "Would you like to select a different node?",
                actions: {
                    Button("Yes, Change Node") {
                        context.dismissWalletError()
                        context.app.pushRoutes(RouteFactory().nestedSettings(route: .node))
                    }

                    Button("Cancel", role: .cancel) {
                        context.dismissWalletError()
                    }
                }
            ).eraseToAny()

        case .noBalance:
            AlertBuilder(
                title: "No Balance",
                message: "Can't send a transaction, when you have no funds.",
                actions: {
                    Button("Receive Funds") {
                        context.dismissWalletError()
                        context.presentationState.wrappedValue = TaggedItem(.receive)
                    }

                    Button("Cancel", role: .cancel) {
                        context.dismissWalletError()
                    }
                }
            ).eraseToAny()
        }
    }
}
