import SwiftUI

struct AboutPresentationContext {
    let alertState: Binding<TaggedItem<AboutAlertState>?>
    let isBetaEnabled: Binding<Bool>
    let dismiss: () -> Void

    func dismissAlert() {
        alertState.wrappedValue = nil
    }

    func presentAlert(_ alert: AboutAlertState) {
        alertState.wrappedValue = TaggedItem(alert)
    }
}

extension AboutAlertState: TaggedAlertPresentable {
    func alert(context: AboutPresentationContext) -> AnyAlertBuilder {
        switch self {
        case .confirmBetaEnable:
            AlertBuilder(
                title: "Enable Beta Features?",
                message: "This will enable experimental features",
                actions: {
                    Button("Enable") {
                        do {
                            try Database().globalFlag().set(
                                key: .betaFeaturesEnabled,
                                value: true
                            )
                            context.isBetaEnabled.wrappedValue = true
                        } catch {
                            context.presentAlert(
                                .betaError(
                                    "Failed to enable beta features: \(error.localizedDescription)"
                                )
                            )
                            return
                        }
                        context.presentAlert(.betaEnabled)
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case .confirmBetaDisable:
            AlertBuilder(
                title: "Disable Beta Features?",
                message: "This will hide experimental features",
                actions: {
                    Button("Disable") {
                        do {
                            try Database().globalFlag().set(
                                key: .betaFeaturesEnabled,
                                value: false
                            )
                            context.isBetaEnabled.wrappedValue = false
                        } catch {
                            context.presentAlert(
                                .betaError(
                                    "Failed to disable beta features: \(error.localizedDescription)"
                                )
                            )
                            return
                        }
                        context.dismissAlert()
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case .betaEnabled:
            AlertBuilder(
                title: "Beta Features Enabled",
                message: "Beta features have been enabled",
                actions: { Button("OK") { context.dismiss() } }
            ).eraseToAny()

        case let .betaError(error):
            AlertBuilder(
                title: "Something went wrong!",
                message: error,
                actions: { Button("OK") { context.dismissAlert() } }
            ).eraseToAny()
        }
    }
}
