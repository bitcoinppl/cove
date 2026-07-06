import SwiftUI

struct AboutPresentationContext {
    let alertState: Binding<TaggedItem<AboutAlertState>?>
    let isBetaEnabled: Binding<Bool>
    let dismiss: () -> Void
    let wipeCloudBackup: @Sendable () -> WipeCloudResult

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

        case .confirmWipeCloud:
            AlertBuilder(
                title: "Wipe Cloud Backup?",
                message: "Deletes all iCloud backup files and resets local backup state",
                actions: {
                    Button("Wipe", role: .destructive) {
                        Task.detached {
                            let result = context.wipeCloudBackup()
                            await MainActor.run {
                                context.presentAlert(.wipeCloudResult(result))
                            }
                        }
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case let .wipeCloudResult(result):
            AlertBuilder(
                title: result.succeeded ? "Cloud Backup Wiped" : "Cloud Backup Wipe Failed",
                message: result.message,
                actions: { Button("OK") { context.dismissAlert() } }
            ).eraseToAny()

        case .confirmResetLocalState:
            AlertBuilder(
                title: "Reset Local Backup State?",
                message: "Clears local keychain and DB backup state but keeps iCloud files intact. Use this to test the recovery flow.",
                actions: {
                    Button("Reset", role: .destructive) {
                        RustCloudBackupManager().debugResetCloudBackupState()
                        context.presentAlert(
                            .resetLocalStateResult(
                                "Local backup state reset. iCloud files are untouched."
                            )
                        )
                    }
                    Button("Cancel", role: .cancel) { context.dismissAlert() }
                }
            ).eraseToAny()

        case let .resetLocalStateResult(message):
            AlertBuilder(
                title: "Local State Reset",
                message: message,
                actions: { Button("OK") { context.dismissAlert() } }
            ).eraseToAny()
        }
    }
}
