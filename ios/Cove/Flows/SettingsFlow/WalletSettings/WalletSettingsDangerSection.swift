import SwiftUI

struct WalletSettingsDangerSection: View {
    let isHotWallet: Bool
    let hasRecoveryWords: Bool
    let hasXprvSecret: Bool
    let showSecretWordsConfirmation: () -> Void
    let showXprvExportWarning: () -> Void
    let prepareDelete: () -> Void

    var body: some View {
        Section(header: Text("Danger Zone")) {
            if isHotWallet, hasRecoveryWords {
                WalletSecretWordsButton(
                    showConfirmation: showSecretWordsConfirmation
                )
            }

            if isHotWallet, hasXprvSecret {
                WalletXprvExportButton(
                    showWarning: showXprvExportWarning
                )
            }

            WalletDeleteButton(action: prepareDelete)
        }
    }
}

struct WalletXprvExportButton: View {
    let showWarning: () -> Void

    var body: some View {
        Button("Export Private Key", action: showWarning)
            .font(.subheadline)
    }
}

struct WalletSecretWordsButton: View {
    let showConfirmation: () -> Void

    var body: some View {
        Button(action: showConfirmation) {
            Text("View Secret Words")
                .font(.subheadline)
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
