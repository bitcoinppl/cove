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
    enum Slot: Equatable {
        case secretWordsConfirmationDialog
        case xprvExportWarningDialog
        case deleteConfirmationDialog
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

    var slot: Slot {
        switch self {
        case .secretWordsConfirmation:
            .secretWordsConfirmationDialog
        case .xprvExportWarning:
            .xprvExportWarningDialog
        case .deleteConfirmation:
            .deleteConfirmationDialog
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

    private var alertIsPresented: Binding<Bool> {
        $presentationState.isPresenting(.alert)
    }

    private var credentialVerificationPresentation:
        Binding<TaggedItem<WalletSettingsPresentationState>?>
    {
        $presentationState.presentedItem(for: .credentialVerification)
    }

    private var xprvRevealPresentation:
        Binding<TaggedItem<WalletSettingsPresentationState>?>
    {
        $presentationState.presentedItem(for: .xprvReveal)
    }

    var body: some View {
        content
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
}

extension Binding where Value == TaggedItem<WalletSettingsPresentationState>? {
    func isPresenting(_ slot: WalletSettingsPresentationState.Slot) -> Binding<Bool> {
        Binding<Bool>(
            get: { wrappedValue?.item.slot == slot },
            set: { isPresented in
                guard !isPresented, wrappedValue?.item.slot == slot else { return }

                wrappedValue = nil
            }
        )
    }

    func presentedItem(
        for slot: WalletSettingsPresentationState.Slot
    ) -> Binding<TaggedItem<WalletSettingsPresentationState>?> {
        Binding(
            get: {
                guard wrappedValue?.item.slot == slot else { return nil }

                return wrappedValue
            },
            set: { newValue in
                if let newValue {
                    wrappedValue = newValue
                } else if wrappedValue?.item.slot == slot {
                    wrappedValue = nil
                }
            }
        )
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
