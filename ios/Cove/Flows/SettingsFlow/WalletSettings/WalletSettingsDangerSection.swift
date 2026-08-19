import SwiftUI

struct WalletSettingsDangerSection: View {
    let isHotWallet: Bool
    let hasRecoveryWords: Bool
    let hasXprvSecret: Bool
    let presentationContext: WalletSettingsPresentationContext
    let showSecretWordsConfirmation: () -> Void
    let showXprvExportWarning: () -> Void
    let prepareDelete: () -> Void
    let presentationCoordinator: PresentationTransitionCoordinator<WalletSettingsPresentationState>

    var body: some View {
        Section(header: Text("Danger Zone")) {
            if isHotWallet, hasRecoveryWords {
                WalletSecretWordsButton(
                    showConfirmation: showSecretWordsConfirmation,
                    showSecretWords: presentationContext.showSecretWords,
                    isPresented: presentationCoordinator.isPresented {
                        $0.slot == .secretWordsConfirmationDialog
                    }
                )
            }

            if isHotWallet, hasXprvSecret {
                WalletXprvExportButton(
                    showWarning: showXprvExportWarning,
                    startExport: presentationContext.startXprvExport,
                    isPresented: presentationCoordinator.isPresented {
                        $0.slot == .xprvExportWarningDialog
                    }
                )
            }

            WalletDeleteButton(
                action: prepareDelete,
                message: presentationContext.deleteConfirmationMessage,
                confirm: presentationContext.confirmInitialDelete,
                presentation: presentationCoordinator.presentedItem {
                    $0.slot == .deleteConfirmationDialog
                }
            )
        }
    }
}

struct WalletXprvExportButton: View {
    let showWarning: () -> Void
    let startExport: (XprvPostVerificationAction) -> Void

    @Binding var isPresented: Bool

    var body: some View {
        Button("Export Private Key", action: showWarning)
            .font(.subheadline)
            .confirmationDialog("Are you sure?", isPresented: $isPresented) {
                Button("Reveal and Copy") { startExport(.reveal) }
                Button("Continue with KeyTeleport") { startExport(.keyTeleport) }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(
                    "Whoever has access to your extended private key, has access to your bitcoin. Please keep it safe, don't show it to anyone."
                )
            }
    }
}

struct WalletSecretWordsButton: View {
    let showConfirmation: () -> Void
    let showSecretWords: () -> Void

    @Binding var isPresented: Bool

    var body: some View {
        Button(action: showConfirmation) {
            Text("View Secret Words")
                .font(.subheadline)
        }
        .confirmationDialog("Are you sure?", isPresented: $isPresented) {
            Button("Show Me", action: showSecretWords)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "Whoever has access to your secret words, has access to your bitcoin. Please keep these safe, don't show them to anyone."
            )
        }
    }
}

struct WalletDeleteButton: View {
    let action: () -> Void
    let message: String
    let confirm: (WalletDeletionConfirmationPlan) -> Void

    @Binding var presentation: TaggedItem<WalletSettingsPresentationState>?

    private var isPresented: Binding<Bool> {
        Binding(
            get: { presentation != nil },
            set: { isPresented in
                if !isPresented { presentation = nil }
            }
        )
    }

    var body: some View {
        Button(action: action) {
            Text("Delete Wallet")
                .foregroundStyle(.red)
                .font(.subheadline)
        }
        .confirmationDialog(
            "Are you sure?",
            isPresented: isPresented,
            presenting: presentation
        ) { presentation in
            if case let .deleteConfirmation(plan) = presentation.item {
                Button("Delete", role: .destructive) {
                    confirm(plan)
                }
                Button("Cancel", role: .cancel) {}
            }
        } message: { _ in
            Text(message)
        }
    }
}
