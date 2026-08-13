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
    @State private var presentationState: TaggedItem<WalletSettingsPresentationState>?
    @State private var accountNumber: UInt32? = nil

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var deleteConfirmationMessage: String {
        manager.rust.deletionWarningMessage()
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
            performXprvExport: performXprvExport
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
                masterFingerprint: manager.rust.masterFingerprint(),
                colorColumns: colorColumns,
                showLabels: showLabels,
                changeName: changeName,
                updateColor: updateColor,
                isHotWallet: metadata.walletType == .hot,
                hasRecoveryWords: manager.hasRecoveryWords(),
                hasXprvSecret: manager.hasXprvSecret() && !auth.isInDecoyMode(),
                showSecretWordsConfirmation: presentSecretWordsConfirmation,
                showXprvExportWarning: presentXprvExportWarning,
                prepareDelete: prepareDelete
            ),
            presentationState: $presentationState
        )
        .navigationTitle(manager.walletMetadata.name)
        .navigationBarTitleDisplayMode(.inline)
        .foregroundColor(.primary)
        .onDisappear(perform: handleDisappear)
        .onAppear(perform: manager.validateMetadata)
        .onChange(of: scenePhase, handleScenePhaseChange)
        .task {
            accountNumber = manager.rust.nonDefaultAccountNumber()
        }
        .scrollContentBackground(.hidden)
    }

    private func deleteWallet() {
        do {
            try manager.rust.deleteWallet()
            dismiss()
        } catch {
            Log.error("Unable to delete wallet: \(error)")
        }
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
            requiredConfirmations: manager.rust.requiredDeletionConfirmations()
        )

        present(.deleteConfirmation(plan))
    }

    private func confirmInitialDelete(_ plan: WalletDeletionConfirmationPlan) {
        guard plan.requiresSecondStep else {
            deleteWallet()
            return
        }

        present(.secondDeleteConfirmation(plan))
    }

    private func confirmSecondDelete(_ plan: WalletDeletionConfirmationPlan) {
        guard plan.requiresFinalStep else {
            deleteWallet()
            return
        }

        present(.finalDeleteConfirmation)
    }

    private func startXprvExport(_ action: XprvPostVerificationAction) {
        guard auth.isAuthEnabled else {
            present(.appLockRequired)
            return
        }

        present(.xprvCredentialVerification(action))
    }

    private func performXprvExport(_ action: XprvPostVerificationAction) {
        presentationState = nil

        switch action {
        case .reveal:
            do {
                try present(.xprvReveal(manager.rust.exposeXprv()))
            } catch {
                Log.error("Unable to reveal private key: \(error)")
                app.alertState = .init(
                    .general(
                        title: "Unable to Reveal Private Key",
                        message: "Cove could not access this wallet's private key."
                    )
                )
            }
        case .keyTeleport:
            app.startKeyTeleportSend(walletId: metadata.id)
        }
    }

    private func dismissXprvReveal() {
        guard presentationState?.item.style == .xprvReveal else { return }

        presentationState = nil
    }

    private func handleDisappear() {
        dismissXprvReveal()
        manager.validateMetadata()
    }

    private func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        guard oldPhase == .active, newPhase != .active else { return }

        dismissXprvReveal()
    }

    private func present(_ state: WalletSettingsPresentationState) {
        presentationState = TaggedItem(state)
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
    let showSecretWordsConfirmation: () -> Void
    let showXprvExportWarning: () -> Void
    let prepareDelete: () -> Void

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
                showSecretWordsConfirmation: showSecretWordsConfirmation,
                showXprvExportWarning: showXprvExportWarning,
                prepareDelete: prepareDelete
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
