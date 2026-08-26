import SwiftUI

struct WalletSettingsView: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.navigate) private var navigate
    @Environment(\.dismiss) private var dismiss

    let manager: WalletManager
    private let colorColumns = Array(repeating: GridItem(.flexible(), spacing: 0), count: 5)

    @State private var cloudBackupManager = CloudBackupManager.shared
    @State private var presentationCoordinator =
        PresentationTransitionCoordinator<WalletSettingsPresentationState>()
    @State private var accountNumber: UInt32? = nil

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var deleteConfirmationMessage: String {
        manager.deletionWarningMessage()
    }

    private var finalDeleteConfirmationMessage: String {
        if cloudBackupManager.isCloudBackupEnabled {
            return "This wallet will be deleted from this device. You can recover it from the Cloud Backup screen, or permanently delete it from there."
        }

        return "This wallet is not backed up and contains funds. You will lose access to these funds forever."
    }

    private var finalDeleteButtonTitle: String {
        cloudBackupManager.isCloudBackupEnabled ? "Delete" : "Delete Forever"
    }

    private var showLabels: Binding<Bool> {
        Binding(
            get: { manager.walletMetadata.showLabels },
            set: { _ in manager.dispatch(action: .toggleShowLabels) }
        )
    }

    private var presentationContext: WalletSettingsPresentationContext {
        WalletSettingsPresentationContext(
            auth: auth,
            walletName: metadata.name,
            deleteConfirmationMessage: deleteConfirmationMessage,
            finalDeleteConfirmationMessage: finalDeleteConfirmationMessage,
            finalDeleteButtonTitle: finalDeleteButtonTitle,
            showSecretWords: showSecretWords,
            startXprvExport: startXprvExport,
            confirmInitialDelete: confirmInitialDelete,
            confirmSecondDelete: confirmSecondDelete,
            deleteWallet: deleteWallet,
            retryDeleteWallet: retryDeleteWallet,
            cancelDeleteWallet: cancelDeleteWallet,
            performXprvExport: performXprvExport,
            loadXprv: manager.exposeXprv
        )
    }

    init(manager: WalletManager) {
        self.manager = manager
    }

    var body: some View {
        WalletSettingsPresentationHost(
            context: presentationContext,
            content: WalletSettingsContent(
                metadata: metadata,
                accountNumber: accountNumber,
                masterFingerprint: manager.masterFingerprint(),
                colorColumns: colorColumns,
                showLabels: showLabels,
                changeName: changeName,
                updateColor: updateColor,
                isHotWallet: metadata.walletType == .hot,
                hasRecoveryWords: manager.hasRecoveryWords(),
                hasXprvSecret: manager.hasXprvSecret() && !auth.isInDecoyMode(),
                presentationContext: presentationContext,
                showSecretWordsConfirmation: presentSecretWordsConfirmation,
                showXprvExportWarning: presentXprvExportWarning,
                prepareDelete: prepareDelete,
                presentationCoordinator: presentationCoordinator
            ),
            presentationCoordinator: presentationCoordinator
        )
        .navigationTitle(manager.walletMetadata.name)
        .navigationBarTitleDisplayMode(.inline)
        .foregroundColor(.primary)
        .onDisappear(perform: handleDisappear)
        .onChange(of: scenePhase, handleScenePhaseChange)
        .task {
            accountNumber = manager.nonDefaultAccountNumber()

            do {
                try await manager.validateMetadata()
            } catch is CancellationError {
                return
            } catch {
                Log.error("Unable to validate wallet metadata: \(error)")
            }
        }
        .scrollContentBackground(.hidden)
    }

    private func deleteWallet() {
        performDeleteWallet(.initial)
    }

    private func retryDeleteWallet(_ attemptId: ShutdownAttemptId) {
        performDeleteWallet(.retry(attemptId))
    }

    private func cancelDeleteWallet(_ attemptId: ShutdownAttemptId) {
        app.cancelWalletDeletionAttempt(attemptId)
        presentationCoordinator.dismissCurrentPresentation()
    }

    private func performDeleteWallet(_ call: WalletDeletionCall) {
        Task { @MainActor in
            do {
                switch call {
                case .initial:
                    try await manager.deleteWallet()
                case let .retry(attemptId):
                    try await manager.retryDeleteWallet(attemptId: attemptId)
                }

                dismiss()
            } catch is CancellationError {
                return
            } catch let WalletManagerError.WalletLifecycle(
                .shutdownBlocked(attemptId, _, _)
            ) {
                present(.deletionShutdownBlocked(attemptId))
            } catch {
                Log.error("Unable to delete wallet: \(error)")
                app.alertState = .init(
                    .general(
                        title: "Unable to Delete Wallet",
                        message: "Cove could not delete this wallet. Try again."
                    )
                )
            }
        }
    }

    private enum WalletDeletionCall {
        case initial
        case retry(ShutdownAttemptId)
    }

    private func changeName() {
        app.pushRoute(Route.settings(.wallet(id: metadata.id, route: .changeName)))
    }

    private func updateColor(_ color: WalletColor) {
        manager.dispatch(action: .updateColor(color))
    }

    private func presentSecretWordsConfirmation() {
        present(.secretWordsConfirmation)
    }

    private func presentXprvExportWarning() {
        present(.xprvExportWarning)
    }

    private func showSecretWords() {
        app.pushRoute(Route.secretWords(manager.walletMetadata.id))
    }

    private func prepareDelete() {
        let plan = WalletDeletionConfirmationPlan(
            requiredConfirmations: manager.requiredDeletionConfirmations()
        )

        present(.deleteConfirmation(plan))
    }

    private func confirmInitialDelete(_ plan: WalletDeletionConfirmationPlan) {
        guard plan.requiresSecondStep else {
            deleteWallet()
            return
        }

        queuePresentation(.secondDeleteConfirmation(plan))
    }

    private func confirmSecondDelete(_ plan: WalletDeletionConfirmationPlan) {
        guard plan.requiresFinalStep else {
            deleteWallet()
            return
        }

        queuePresentation(.finalDeleteConfirmation)
    }

    private func startXprvExport(_ action: XprvPostVerificationAction) {
        guard auth.isAuthEnabled else {
            queuePresentation(.appLockRequired)
            return
        }

        queuePresentation(.xprvCredentialVerification(action))
    }

    private func performXprvExport(_ action: XprvPostVerificationAction) {
        switch action {
        case .reveal:
            guard scenePhase == .active else {
                presentationCoordinator.dismissCurrentPresentation()
                return
            }

            queuePresentation(.xprvReveal)
        case .keyTeleport:
            presentationCoordinator.dismissCurrentPresentation()
            app.startKeyTeleportSend(walletId: metadata.id)
        }
    }

    private func clearXprvReveal() {
        presentationCoordinator.discard { $0.slot == .xprvReveal }
    }

    private func handleDisappear() {
        clearXprvReveal()

        Task { @MainActor in
            do {
                try await manager.validateMetadata()
            } catch is CancellationError {
                return
            } catch {
                Log.error("Unable to validate wallet metadata: \(error)")
            }
        }
    }

    private func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        guard oldPhase == .active, newPhase != .active else { return }

        clearXprvReveal()
    }

    private func present(_ state: WalletSettingsPresentationState) {
        presentationCoordinator.present(state)
    }

    private func queuePresentation(_ state: WalletSettingsPresentationState) {
        presentationCoordinator.transition(to: state)
    }
}

private struct WalletSettingsContent: View {
    let metadata: WalletMetadata
    let accountNumber: UInt32?
    let masterFingerprint: String?
    let colorColumns: [GridItem]
    @Binding var showLabels: Bool
    let changeName: () -> Void
    let updateColor: (WalletColor) -> Void
    let isHotWallet: Bool
    let hasRecoveryWords: Bool
    let hasXprvSecret: Bool
    let presentationContext: WalletSettingsPresentationContext
    let showSecretWordsConfirmation: () -> Void
    let showXprvExportWarning: () -> Void
    let prepareDelete: () -> Void
    let presentationCoordinator: PresentationTransitionCoordinator<WalletSettingsPresentationState>

    var body: some View {
        List {
            WalletSettingsInformationSection(
                metadata: metadata,
                accountNumber: accountNumber,
                masterFingerprint: masterFingerprint
            )
            WalletSettingsPreferencesSection(
                metadata: metadata,
                colorColumns: colorColumns,
                showLabels: $showLabels,
                changeName: changeName,
                updateColor: updateColor
            )
            WalletSettingsDangerSection(
                isHotWallet: isHotWallet,
                hasRecoveryWords: hasRecoveryWords,
                hasXprvSecret: hasXprvSecret,
                presentationContext: presentationContext,
                showSecretWordsConfirmation: showSecretWordsConfirmation,
                showXprvExportWarning: showXprvExportWarning,
                prepareDelete: prepareDelete,
                presentationCoordinator: presentationCoordinator
            )
        }
    }
}

#Preview {
    AsyncPreview {
        WalletSettingsView(manager: WalletManager(preview: .only))
            .environment(AppManager.shared)
            .environment(AuthManager.shared)
            .environment(\.navigate) { _ in
                ()
            }
            .background(Color(UIColor.systemGroupedBackground))
    }
}
