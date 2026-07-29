import SwiftUI

struct WalletSettingsDangerSection: View {
    let walletName: String
    let isHotWallet: Bool
    let hasRecoveryWords: Bool
    let hasXprvSecret: Bool
    let deleteConfirmationMessage: String
    let finalDeleteConfirmationMessage: String
    let finalDeleteButtonTitle: String
    @Binding var requiredConfirmations: UInt8
    @Binding var showingSecretWordsConfirmation: Bool
    @Binding var showingXprvExportWarning: Bool
    @Binding var showingDeleteConfirmation: Bool
    @Binding var showingSecondDeleteConfirmation: Bool
    @Binding var showingFinalDeleteConfirmation: Bool
    let showSecretWords: () -> Void
    let startXprvExport: (XprvPostVerificationAction) -> Void
    let prepareDelete: () -> Void
    let deleteWallet: () -> Void

    var body: some View {
        Section(header: Text("Danger Zone")) {
            if isHotWallet, hasRecoveryWords {
                WalletSecretWordsButton(
                    isPresented: $showingSecretWordsConfirmation,
                    showSecretWords: showSecretWords
                )
            }

            if isHotWallet, hasXprvSecret {
                WalletXprvExportButton(
                    showingWarning: $showingXprvExportWarning,
                    export: startXprvExport
                )
            }

            WalletFinalDeleteConfirmationHost(
                isPresented: $showingFinalDeleteConfirmation,
                buttonTitle: finalDeleteButtonTitle,
                message: finalDeleteConfirmationMessage,
                delete: deleteWallet
            ) {
                WalletSecondDeleteConfirmationHost(
                    isPresented: $showingSecondDeleteConfirmation,
                    requiredConfirmations: requiredConfirmations,
                    walletName: walletName,
                    showFinalConfirmation: { showingFinalDeleteConfirmation = true },
                    delete: deleteWallet
                ) {
                    WalletInitialDeleteConfirmationHost(
                        isPresented: $showingDeleteConfirmation,
                        requiredConfirmations: requiredConfirmations,
                        message: deleteConfirmationMessage,
                        showSecondConfirmation: { showingSecondDeleteConfirmation = true },
                        delete: deleteWallet
                    ) {
                        WalletDeleteButton(action: prepareDelete)
                    }
                }
            }
        }
    }
}

struct WalletXprvExportButton: View {
    @Binding var showingWarning: Bool
    let export: (XprvPostVerificationAction) -> Void

    var body: some View {
        Button("Export Private Key") {
            showingWarning = true
        }
        .font(.subheadline)
        .confirmationDialog("Are you sure?", isPresented: $showingWarning) {
            Button("Reveal and Copy") { export(.reveal) }
            Button("Continue with KeyTeleport") { export(.keyTeleport) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "Whoever has access to your extended private key, has access to your bitcoin. Please keep it safe, don't show it to anyone."
            )
        }
    }
}

struct WalletSecretWordsButton: View {
    @Binding var isPresented: Bool
    let showSecretWords: () -> Void

    var body: some View {
        Button {
            isPresented = true
        } label: {
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

    var body: some View {
        Button(action: action) {
            Text("Delete Wallet")
                .foregroundStyle(.red)
                .font(.subheadline)
        }
    }
}

struct WalletInitialDeleteConfirmationHost<Content: View>: View {
    @Binding var isPresented: Bool
    let requiredConfirmations: UInt8
    let message: String
    let showSecondConfirmation: () -> Void
    let delete: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .confirmationDialog("Are you sure?", isPresented: $isPresented) {
                Button("Delete", role: .destructive, action: confirmDelete)
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(message)
            }
    }

    private func confirmDelete() {
        if requiredConfirmations >= 2 {
            showSecondConfirmation()
        } else {
            delete()
        }
    }
}

struct WalletSecondDeleteConfirmationHost<Content: View>: View {
    @Binding var isPresented: Bool
    let requiredConfirmations: UInt8
    let walletName: String
    let showFinalConfirmation: () -> Void
    let delete: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .alert("Confirm Deletion", isPresented: $isPresented) {
                Button("Delete", role: .destructive, action: confirmDelete)
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Are you sure you want to delete '\(walletName)'?")
            }
    }

    private func confirmDelete() {
        if requiredConfirmations >= 3 {
            showFinalConfirmation()
        } else {
            delete()
        }
    }
}

struct WalletFinalDeleteConfirmationHost<Content: View>: View {
    @Binding var isPresented: Bool
    let buttonTitle: String
    let message: String
    let delete: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .alert("Final Warning", isPresented: $isPresented) {
                Button(buttonTitle, role: .destructive, action: delete)
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(message)
            }
    }
}
