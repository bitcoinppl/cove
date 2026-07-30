import SwiftUI

enum XprvPostVerificationAction: Equatable {
    case reveal
    case keyTeleport
}

enum WalletDeletionConfirmationPlan: Equatable {
    case oneStep
    case twoSteps
    case threeSteps

    init(requiredConfirmations: UInt8) {
        switch requiredConfirmations {
        case 0 ... 1:
            self = .oneStep
        case 2:
            self = .twoSteps
        default:
            self = .threeSteps
        }
    }

    var requiresSecondStep: Bool {
        self != .oneStep
    }

    var requiresFinalStep: Bool {
        self == .threeSteps
    }
}

enum WalletSettingsPresentationState: Equatable {
    enum Style: Equatable {
        case confirmationDialog
        case alert
        case credentialVerification
        case xprvReveal
    }

    case secretWordsConfirmation
    case xprvExportWarning
    case deleteConfirmation(WalletDeletionConfirmationPlan)
    case secondDeleteConfirmation(WalletDeletionConfirmationPlan)
    case finalDeleteConfirmation
    case appLockRequired
    case xprvCredentialVerification(XprvPostVerificationAction)
    case xprvReveal(String)

    var style: Style {
        switch self {
        case .secretWordsConfirmation, .xprvExportWarning, .deleteConfirmation:
            .confirmationDialog
        case .secondDeleteConfirmation, .finalDeleteConfirmation, .appLockRequired:
            .alert
        case .xprvCredentialVerification:
            .credentialVerification
        case .xprvReveal:
            .xprvReveal
        }
    }
}

struct WalletSettingsPresentationContext {
    let auth: AuthManager
    let walletName: String
    let deleteConfirmationMessage: String
    let finalDeleteConfirmationMessage: String
    let finalDeleteButtonTitle: String
    let showSecretWords: () -> Void
    let startXprvExport: (XprvPostVerificationAction) -> Void
    let confirmInitialDelete: (WalletDeletionConfirmationPlan) -> Void
    let confirmSecondDelete: (WalletDeletionConfirmationPlan) -> Void
    let deleteWallet: () -> Void
    let performXprvExport: (XprvPostVerificationAction) -> Void

    func alertTitle(for state: WalletSettingsPresentationState?) -> String {
        switch state {
        case .secondDeleteConfirmation:
            "Confirm Deletion"
        case .finalDeleteConfirmation:
            "Final Warning"
        case .appLockRequired:
            "App Lock Required"
        default:
            "Alert"
        }
    }
}

struct WalletSettingsPresentationHost<Content: View>: View {
    let context: WalletSettingsPresentationContext
    let content: Content

    @Binding var presentationState: TaggedItem<WalletSettingsPresentationState>?

    private var confirmationDialogIsPresented: Binding<Bool> {
        isPresenting(.confirmationDialog)
    }

    private var alertIsPresented: Binding<Bool> {
        isPresenting(.alert)
    }

    private var credentialVerificationPresentation:
        Binding<TaggedItem<WalletSettingsPresentationState>?>
    {
        presentedItem(for: .credentialVerification)
    }

    private var xprvRevealPresentation:
        Binding<TaggedItem<WalletSettingsPresentationState>?>
    {
        presentedItem(for: .xprvReveal)
    }

    var body: some View {
        content
            .confirmationDialog(
                "Are you sure?",
                isPresented: confirmationDialogIsPresented,
                presenting: presentationState
            ) { presentation in
                WalletSettingsConfirmationDialogActions(
                    presentation: presentation.item,
                    context: context
                )
            } message: { presentation in
                WalletSettingsConfirmationDialogMessage(
                    presentation: presentation.item,
                    context: context
                )
            }
            .alert(
                context.alertTitle(for: presentationState?.item),
                isPresented: alertIsPresented,
                presenting: presentationState
            ) { presentation in
                WalletSettingsAlertActions(
                    presentation: presentation.item,
                    context: context
                )
            } message: { presentation in
                WalletSettingsAlertMessage(
                    presentation: presentation.item,
                    context: context
                )
            }
            .fullScreenCover(item: credentialVerificationPresentation) { presentation in
                WalletSettingsCredentialVerificationDestination(
                    presentation: presentation.item,
                    context: context
                )
            }
            .sheet(item: xprvRevealPresentation) { presentation in
                WalletSettingsXprvRevealDestination(presentation: presentation.item)
            }
    }

    private func isPresenting(_ style: WalletSettingsPresentationState.Style) -> Binding<Bool> {
        Binding(
            get: { presentationState?.item.style == style },
            set: { isPresented in
                guard !isPresented, presentationState?.item.style == style else { return }

                presentationState = nil
            }
        )
    }

    private func presentedItem(
        for style: WalletSettingsPresentationState.Style
    ) -> Binding<TaggedItem<WalletSettingsPresentationState>?> {
        Binding(
            get: {
                guard presentationState?.item.style == style else { return nil }

                return presentationState
            },
            set: { newValue in
                if let newValue {
                    presentationState = newValue
                } else if presentationState?.item.style == style {
                    presentationState = nil
                }
            }
        )
    }
}

private struct WalletSettingsConfirmationDialogActions: View {
    let presentation: WalletSettingsPresentationState
    let context: WalletSettingsPresentationContext

    var body: some View {
        switch presentation {
        case .secretWordsConfirmation:
            WalletSecretWordsConfirmationActions(showSecretWords: context.showSecretWords)
        case .xprvExportWarning:
            WalletXprvExportWarningActions(startExport: context.startXprvExport)
        case let .deleteConfirmation(plan):
            WalletInitialDeleteConfirmationActions(
                plan: plan,
                confirm: context.confirmInitialDelete
            )
        default:
            EmptyView()
        }
    }
}

private struct WalletSecretWordsConfirmationActions: View {
    let showSecretWords: () -> Void

    var body: some View {
        Button("Show Me", action: showSecretWords)
        Button("Cancel", role: .cancel) {}
    }
}

private struct WalletXprvExportWarningActions: View {
    let startExport: (XprvPostVerificationAction) -> Void

    var body: some View {
        Button("Reveal and Copy") { startExport(.reveal) }
        Button("Continue with KeyTeleport") { startExport(.keyTeleport) }
        Button("Cancel", role: .cancel) {}
    }
}

private struct WalletInitialDeleteConfirmationActions: View {
    let plan: WalletDeletionConfirmationPlan
    let confirm: (WalletDeletionConfirmationPlan) -> Void

    var body: some View {
        Button("Delete", role: .destructive) {
            confirm(plan)
        }
        Button("Cancel", role: .cancel) {}
    }
}

private struct WalletSettingsConfirmationDialogMessage: View {
    let presentation: WalletSettingsPresentationState
    let context: WalletSettingsPresentationContext

    var body: some View {
        switch presentation {
        case .secretWordsConfirmation:
            Text(
                "Whoever has access to your secret words, has access to your bitcoin. Please keep these safe, don't show them to anyone."
            )
        case .xprvExportWarning:
            Text(
                "Whoever has access to your extended private key, has access to your bitcoin. Please keep it safe, don't show it to anyone."
            )
        case .deleteConfirmation:
            Text(context.deleteConfirmationMessage)
        default:
            EmptyView()
        }
    }
}

private struct WalletSettingsAlertActions: View {
    let presentation: WalletSettingsPresentationState
    let context: WalletSettingsPresentationContext

    var body: some View {
        switch presentation {
        case let .secondDeleteConfirmation(plan):
            Button("Delete", role: .destructive) {
                context.confirmSecondDelete(plan)
            }
            Button("Cancel", role: .cancel) {}
        case .finalDeleteConfirmation:
            Button(context.finalDeleteButtonTitle, role: .destructive, action: context.deleteWallet)
            Button("Cancel", role: .cancel) {}
        case .appLockRequired:
            Button("OK", role: .cancel) {}
        default:
            EmptyView()
        }
    }
}

private struct WalletSettingsAlertMessage: View {
    let presentation: WalletSettingsPresentationState
    let context: WalletSettingsPresentationContext

    var body: some View {
        switch presentation {
        case .secondDeleteConfirmation:
            Text("Are you sure you want to delete '\(context.walletName)'?")
        case .finalDeleteConfirmation:
            Text(context.finalDeleteConfirmationMessage)
        case .appLockRequired:
            Text("Enable a PIN or biometric app lock before exporting a private key.")
        default:
            EmptyView()
        }
    }
}

private struct WalletSettingsCredentialVerificationDestination: View {
    let presentation: WalletSettingsPresentationState
    let context: WalletSettingsPresentationContext

    var body: some View {
        if case let .xprvCredentialVerification(action) = presentation {
            WalletXprvCredentialVerification(
                auth: context.auth,
                action: action,
                perform: context.performXprvExport
            )
        }
    }
}

private struct WalletSettingsXprvRevealDestination: View {
    let presentation: WalletSettingsPresentationState

    var body: some View {
        if case let .xprvReveal(xprv) = presentation {
            XprvRevealSheet(xprv: xprv)
        }
    }
}

private struct WalletXprvCredentialVerification: View {
    let auth: AuthManager
    let action: XprvPostVerificationAction
    let perform: (XprvPostVerificationAction) -> Void

    @State private var succeeded = false

    var body: some View {
        MainCredentialVerificationView(auth: auth) {
            succeeded = true
        }
        .onDisappear(perform: completeIfVerified)
    }

    private func completeIfVerified() {
        guard succeeded else { return }

        succeeded = false
        perform(action)
    }
}
