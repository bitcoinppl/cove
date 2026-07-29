//
//  SecretWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/22/24.
//

import SwiftUI

struct SecretWordsScreen: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth

    let id: WalletId

    // private
    @State var words: Mnemonic?
    @State var errorMessage: String?
    @State private var pendingSensitiveAction: SecretWordsSensitiveAction?
    @State private var showSeedQrSheet = false
    @State private var showingKeyTeleportCredentialVerification = false
    @State private var keyTeleportCredentialVerificationSucceeded = false
    @State private var showingAppLockRequired = false

    let rowHeight = 30.0
    private let numberOfColumns = 2

    var numberOfRows: Int {
        (words?.words().count ?? 24) / numberOfColumns
    }

    private var showingSensitiveActionConfirmation: Binding<Bool> {
        Binding(
            get: { pendingSensitiveAction != nil },
            set: { isPresented in
                if !isPresented {
                    pendingSensitiveAction = nil
                }
            }
        )
    }

    var body: some View {
        SecretWordsLayout(
            sizeCategory: sizeCategory,
            words: words,
            errorMessage: errorMessage,
            rowHeight: rowHeight,
            numberOfRows: numberOfRows,
            numberOfColumns: numberOfColumns
        )
        .onAppear(perform: loadWords)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .adaptiveToolbarStyle()
        .toolbar {
            SecretWordsOptionsToolbar(
                isInDecoyMode: auth.isInDecoyMode(),
                pendingAction: pendingSensitiveAction,
                isConfirmationPresented: showingSensitiveActionConfirmation,
                presentConfirmation: presentConfirmation,
                performSensitiveAction: performSensitiveAction
            )
        }
        .sheet(isPresented: $showSeedQrSheet) {
            SeedQrPresentation(words: words)
        }
        .fullScreenCover(
            isPresented: $showingKeyTeleportCredentialVerification,
            onDismiss: handleCredentialVerificationDismiss
        ) {
            SecretWordsCredentialVerification(
                auth: auth,
                succeeded: $keyTeleportCredentialVerificationSucceeded
            )
        }
        .alert("App Lock Required", isPresented: $showingAppLockRequired) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Enable a PIN or biometric app lock before sending secret words with KeyTeleport.")
        }
        .background(SecretWordsPatternBackground())
        .background(Color.midnightBlue)
    }

    private func presentConfirmation(for action: SecretWordsSensitiveAction) {
        // wait for Menu dismissal so the action sheet can anchor to the toolbar button
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            guard pendingSensitiveAction == nil, !showSeedQrSheet else { return }
            pendingSensitiveAction = action
        }
    }

    private func performSensitiveAction(_ action: SecretWordsSensitiveAction) {
        pendingSensitiveAction = nil

        switch action {
        case .seedQr:
            showSeedQrSheet = true
        case .keyTeleport:
            guard auth.isAuthEnabled else {
                showingAppLockRequired = true
                return
            }

            keyTeleportCredentialVerificationSucceeded = false
            showingKeyTeleportCredentialVerification = true
        }
    }

    private func handleCredentialVerificationDismiss() {
        defer { keyTeleportCredentialVerificationSucceeded = false }
        guard keyTeleportCredentialVerificationSucceeded else { return }

        app.startKeyTeleportSend(walletId: id)
    }

    private func loadWords() {
        auth.lock()
        guard words == nil else { return }

        do {
            words = try Mnemonic(id: id)
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

#Preview("12") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twelve))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}

#Preview("24") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twentyFour))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
