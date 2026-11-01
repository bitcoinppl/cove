//
//  SecretWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/22/24.
//

import SwiftUI

/// Whether the recovery words are loaded
///
/// The mnemonic exists only in `revealed`, which is reached only after the user opens the app lock
/// with a credential, so no word is in UI state while the lock cover is up
enum SecretWordsRevealState {
    case unrequested
    case awaitingCredential(CredentialUnlockRequest)
    case revealed(Mnemonic, lockGeneration: UInt64)
    case failed(String)

    var errorMessage: String? {
        if case let .failed(message) = self { message } else { nil }
    }
}

struct SecretWordsScreen: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth

    let id: WalletId

    // private
    @State private var revealState = SecretWordsRevealState.unrequested
    @State private var pendingSensitiveAction: SecretWordsSensitiveAction?
    @State private var showSeedQrSheet = false
    @State private var showingKeyTeleportCredentialVerification = false
    @State private var keyTeleportCredentialVerificationSucceeded = false
    @State private var showingAppLockRequired = false

    let rowHeight = 30.0
    private let numberOfColumns = 2

    var numberOfRows: Int {
        (accessibleWords?.words().count ?? 24) / numberOfColumns
    }

    /// The words, only while the lock that guarded them has not engaged since they loaded
    ///
    /// `reconcileRevealState` drops them after the update that ends access, so rendering checks
    /// access itself rather than waiting for it
    private var accessibleWords: Mnemonic? {
        guard case let .revealed(words, lockGeneration) = revealState else { return nil }
        guard auth.lockState == .unlocked, auth.lockGeneration == lockGeneration else { return nil }
        return words
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
            words: accessibleWords,
            errorMessage: revealState.errorMessage,
            rowHeight: rowHeight,
            numberOfRows: numberOfRows,
            numberOfColumns: numberOfColumns
        )
        .onAppear(perform: reconcileRevealState)
        .onChange(of: auth.lockState) { reconcileRevealState() }
        .onChange(of: auth.lockGeneration) { reconcileRevealState() }
        .onChange(of: auth.credentialUnlockGeneration) { reconcileRevealState() }
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
            SeedQrPresentation(words: accessibleWords)
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

    /// Ask for a credential on appear, load only once it is presented, and drop the words when the lock engages again
    private func reconcileRevealState() {
        switch revealState {
        case .unrequested:
            requestCredentialOrLoad()

        case let .awaitingCredential(request):
            switch auth.credentialUnlockStatus(request) {
            case .satisfied:
                loadWords()

            // a lock after the credential that the quick-return grace released is not a credential, so ask again
            case .unlockedWithoutCredential:
                requestCredentialOrLoad()

            // a mode switch resets the app, so this screen is on its way out and must stay empty
            case .pending, .superseded:
                break
            }

        case .revealed:
            guard accessibleWords == nil else { return }

            showSeedQrSheet = false
            requestCredentialOrLoad()

        case .failed:
            break
        }
    }

    private func requestCredentialOrLoad() {
        if let request = auth.requestCredentialUnlock() {
            revealState = .awaitingCredential(request)
        } else {
            loadWords()
        }
    }

    private func loadWords() {
        do {
            revealState = try .revealed(Mnemonic(id: id), lockGeneration: auth.lockGeneration)
        } catch {
            revealState = .failed(error.localizedDescription)
        }
    }
}

#Preview("12") {
    SecretWordsLayoutPreview(words: Mnemonic.preview(numberOfBip39Words: .twelve))
}

#Preview("24") {
    SecretWordsLayoutPreview(words: Mnemonic.preview(numberOfBip39Words: .twentyFour))
}

/// previews render the layout directly because the screen only loads words after a credential
private struct SecretWordsLayoutPreview: View {
    let words: Mnemonic

    var body: some View {
        SecretWordsLayout(
            sizeCategory: .medium,
            words: words,
            errorMessage: nil,
            rowHeight: 30,
            numberOfRows: words.words().count / 2,
            numberOfColumns: 2
        )
        .background(SecretWordsPatternBackground())
        .background(Color.midnightBlue)
    }
}
