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
    case deletionShutdownBlocked(ShutdownAttemptId)
    case appLockRequired
    case xprvCredentialVerification(XprvPostVerificationAction)
    case xprvReveal

    var slot: Slot {
        switch self {
        case .secretWordsConfirmation:
            .secretWordsConfirmationDialog
        case .xprvExportWarning:
            .xprvExportWarningDialog
        case .deleteConfirmation:
            .deleteConfirmationDialog
        case .secondDeleteConfirmation,
             .finalDeleteConfirmation,
             .deletionShutdownBlocked,
             .appLockRequired:
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
    let retryDeleteWallet: (ShutdownAttemptId) -> Void
    let cancelDeleteWallet: (ShutdownAttemptId) -> Void
    let performXprvExport: (XprvPostVerificationAction) -> Void
    let loadXprv: () throws -> String

    func alertTitle(for state: WalletSettingsPresentationState?) -> String {
        switch state {
        case .secondDeleteConfirmation:
            "Confirm Deletion"
        case .finalDeleteConfirmation:
            "Final Warning"
        case .deletionShutdownBlocked:
            "Wallet Shutdown Is Blocked"
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
    let presentationCoordinator: PresentationTransitionCoordinator<WalletSettingsPresentationState>

    private var presentationState: TaggedItem<WalletSettingsPresentationState>? {
        presentationCoordinator.currentPresentation
    }

    private var alertIsPresented: Binding<Bool> {
        presentationCoordinator.isPresented { $0.slot == .alert }
    }

    private var credentialVerificationPresentation:
        Binding<TaggedItem<WalletSettingsPresentationState>?>
    {
        presentationCoordinator.presentedItem { $0.slot == .credentialVerification }
    }

    private var xprvRevealPresentation:
        Binding<TaggedItem<WalletSettingsPresentationState>?>
    {
        presentationCoordinator.presentedItem { $0.slot == .xprvReveal }
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
                WalletSettingsXprvRevealDestination(
                    loadXprv: context.loadXprv
                )
                .id(presentation.id)
            }
            .presentationTransitionHost(presentationCoordinator)
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
        case let .deletionShutdownBlocked(attemptId):
            Button("Retry") {
                context.retryDeleteWallet(attemptId)
            }
            Button("Cancel", role: .cancel) {
                context.cancelDeleteWallet(attemptId)
            }
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
        case .deletionShutdownBlocked:
            Text("Cove could not stop all wallet work. Retry or cancel the deletion.")
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

@MainActor
@Observable
final class WalletSettingsXprvRevealPresentation {
    enum State {
        case awaitingLoad
        case loading
        case loaded(String)
        case failed
        case ended
    }

    private(set) var state = State.awaitingLoad

    func load(using loadXprv: () throws -> String) {
        guard case .awaitingLoad = state else { return }

        state = .loading

        do {
            state = try .loaded(loadXprv())
        } catch {
            Log.error("Unable to reveal private key: \(error)")
            state = .failed
        }
    }

    func end() {
        state = .ended
    }
}

private struct WalletSettingsXprvRevealDestination: View {
    @Environment(\.dismiss) private var dismiss

    let loadXprv: () throws -> String

    @State private var presentation = WalletSettingsXprvRevealPresentation()

    var body: some View {
        Group {
            switch presentation.state {
            case .awaitingLoad, .loading:
                ProgressView()
            case let .loaded(xprv):
                XprvRevealSheet(xprv: xprv)
            case .failed:
                NavigationStack {
                    ContentUnavailableView(
                        "Unable to Reveal Private Key",
                        systemImage: "exclamationmark.triangle",
                        description: Text("Cove could not access this wallet's private key.")
                    )
                    .navigationTitle("Private Key")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        XprvRevealToolbar(done: dismiss.callAsFunction)
                    }
                }
            case .ended:
                Color.clear
                    .task {
                        dismiss()
                    }
            }
        }
        .task {
            presentation.load(using: loadXprv)
        }
        .onDisappear(perform: presentation.end)
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
