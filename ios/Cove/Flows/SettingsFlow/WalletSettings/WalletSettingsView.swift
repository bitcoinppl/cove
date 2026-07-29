import SwiftUI

enum XprvPostVerificationAction {
    case reveal
    case keyTeleport
}

struct WalletSettingsView: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.navigate) private var navigate
    @Environment(\.dismiss) private var dismiss

    let manager: WalletManager

    @State private var cloudBackupManager = CloudBackupManager.shared
    @State private var showingDeleteConfirmation = false
    @State private var showingSecretWordsConfirmation = false
    @State private var showingSecondDeleteConfirmation = false
    @State private var showingFinalDeleteConfirmation = false
    @State private var showingXprvExportWarning = false
    @State private var showingXprvCredentialVerification = false
    @State private var xprvCredentialVerificationSucceeded = false
    @State private var pendingXprvAction: XprvPostVerificationAction?
    @State private var revealedXprv: String?
    @State private var showingXprvReveal = false
    @State private var showingAppLockRequired = false
    @State private var requiredConfirmations: UInt8 = 1
    @State private var accountNumber: UInt32? = nil

    init(manager: WalletManager) {
        self.manager = manager
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var deleteConfirmationMessage: String {
        manager.rust.deletionWarningMessage()
    }

    var finalDeleteConfirmationMessage: String {
        if cloudBackupManager.isCloudBackupEnabled {
            return "This wallet will be deleted from this device. You can recover it from the Cloud Backup screen, or permanently delete it from there."
        }

        return "This wallet is not backed up and contains funds. You will lose access to these funds forever."
    }

    var finalDeleteButtonTitle: String {
        cloudBackupManager.isCloudBackupEnabled ? "Delete" : "Delete Forever"
    }

    private var showLabels: Binding<Bool> {
        Binding(
            get: { manager.walletMetadata.showLabels },
            set: { _ in manager.dispatch(action: .toggleShowLabels) }
        )
    }

    let colorColumns = Array(repeating: GridItem(.flexible(), spacing: 0), count: 5)

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

    private func showSecretWords() {
        app.pushRoute(Route.secretWords(manager.walletMetadata.id))
    }

    private func prepareDelete() {
        requiredConfirmations = manager.rust.requiredDeletionConfirmations()
        showingDeleteConfirmation = true
    }

    private func startXprvExport(_ action: XprvPostVerificationAction) {
        guard auth.isAuthEnabled else {
            showingAppLockRequired = true
            return
        }

        clearRevealedXprv()
        pendingXprvAction = action
        xprvCredentialVerificationSucceeded = false
        showingXprvCredentialVerification = true
    }

    private func performXprvExport(_ action: XprvPostVerificationAction) {
        switch action {
        case .reveal:
            do {
                revealedXprv = try manager.rust.exposeXprv()
                showingXprvReveal = true
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

    private func clearRevealedXprv() {
        revealedXprv = nil
        showingXprvReveal = false
    }

    private func handleDisappear() {
        clearRevealedXprv()
        manager.validateMetadata()
    }

    private func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        guard oldPhase == .active, newPhase != .active else { return }

        clearRevealedXprv()
    }

    private func handleXprvVerificationDismiss() {
        defer {
            pendingXprvAction = nil
            xprvCredentialVerificationSucceeded = false
        }
        guard xprvCredentialVerificationSucceeded, let pendingXprvAction else { return }

        performXprvExport(pendingXprvAction)
    }

    var body: some View {
        List {
            WalletSettingsInformationSection(
                metadata: metadata,
                accountNumber: accountNumber,
                masterFingerprint: manager.rust.masterFingerprint()
            )
            WalletSettingsPreferencesSection(
                metadata: metadata,
                colorColumns: colorColumns,
                showLabels: showLabels,
                changeName: changeName,
                updateColor: updateColor
            )
            WalletSettingsDangerSection(
                walletName: metadata.name,
                isHotWallet: metadata.walletType == .hot,
                hasRecoveryWords: manager.hasRecoveryWords(),
                hasXprvSecret: manager.hasXprvSecret() && !auth.isInDecoyMode(),
                deleteConfirmationMessage: deleteConfirmationMessage,
                finalDeleteConfirmationMessage: finalDeleteConfirmationMessage,
                finalDeleteButtonTitle: finalDeleteButtonTitle,
                requiredConfirmations: $requiredConfirmations,
                showingSecretWordsConfirmation: $showingSecretWordsConfirmation,
                showingXprvExportWarning: $showingXprvExportWarning,
                showingDeleteConfirmation: $showingDeleteConfirmation,
                showingSecondDeleteConfirmation: $showingSecondDeleteConfirmation,
                showingFinalDeleteConfirmation: $showingFinalDeleteConfirmation,
                showSecretWords: showSecretWords,
                startXprvExport: startXprvExport,
                prepareDelete: prepareDelete,
                deleteWallet: deleteWallet
            )
        }
        .navigationTitle(manager.walletMetadata.name)
        .navigationBarTitleDisplayMode(.inline)
        .foregroundColor(.primary)
        .onDisappear(perform: handleDisappear)
        .onAppear(perform: manager.validateMetadata)
        .onChange(of: scenePhase, handleScenePhaseChange)
        .task {
            accountNumber = manager.rust.nonDefaultAccountNumber()
        }
        .fullScreenCover(
            isPresented: $showingXprvCredentialVerification,
            onDismiss: handleXprvVerificationDismiss
        ) {
            WalletXprvCredentialVerification(
                auth: auth,
                succeeded: $xprvCredentialVerificationSucceeded
            )
        }
        .sheet(
            isPresented: $showingXprvReveal,
            onDismiss: clearRevealedXprv
        ) {
            XprvRevealSheet(xprv: $revealedXprv)
        }
        .alert("App Lock Required", isPresented: $showingAppLockRequired) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Enable a PIN or biometric app lock before exporting a private key.")
        }
        .scrollContentBackground(.hidden)
    }
}

private struct WalletXprvCredentialVerification: View {
    let auth: AuthManager
    @Binding var succeeded: Bool

    var body: some View {
        MainCredentialVerificationView(auth: auth) {
            succeeded = true
        }
    }
}

#Preview {
    AsyncPreview {
        WalletSettingsView(manager: WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
            .environment(AuthManager.shared)
            .environment(\.navigate) { _ in
                ()
            }
            .background(Color(UIColor.systemGroupedBackground))
    }
}
