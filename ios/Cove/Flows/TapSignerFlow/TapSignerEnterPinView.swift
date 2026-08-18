//
//  TapSignerEnterPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import CoveCore
import SwiftUI

struct TapSignerEnterPin: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let action: AfterPinAction

    var message: String {
        action.userMessage()
    }

    // private
    @State private var pin = ""
    @State private var errorMessage: String?
    @FocusState private var isFocused

    /// confirmed pin is correct, now run the action
    private func runAction(_ nfc: TapSignerNFC, _ pin: String) {
        switch action {
        case .derive: deriveAction(nfc, pin)
        case .change:
            manager.navigate(
                to:
                .newPin(
                    TapSignerNewPinArgs(
                        tapSigner: tapSigner,
                        startingPin: pin,
                        chainCode: .none,
                        action: .change
                    )
                )
            )
        case .backup:
            backupAction(nfc, pin)
        case let .sign(psbt):
            signAction(nfc, psbt, pin)
        }
    }

    private func deriveAction(_ nfc: TapSignerNFC, _ pin: String) {
        Task {
            switch await nfc.derive(pin: pin) {
            case let .success(deriveInfo):
                manager.enteredPin = nil
                manager.resetRoute(to: .importSuccess(tapSigner, deriveInfo))
            case let .failure(error):
                if error.isAuthError() {
                    app.sheetState = nil
                    app.alertState = .init(.tapSignerWrongPin(tapSigner: tapSigner, action: .derive))
                } else {
                    app.alertState = .init(
                        .tapSignerDeriveFailed(
                            message: "TapSigner import failed. Please try again."
                        )
                    )
                }
            }

            await MainActor.run { self.pin = "" }
        }
    }

    private func backupAction(_ nfc: TapSignerNFC, _ pin: String) {
        Task {
            switch await nfc.backup(pin: pin) {
            case let .success(backup):
                let _ = app.saveTapSignerBackup(tapSigner, backup)
                manager.enteredPin = nil
                await MainActor.run {
                    self.pin = ""
                    app.sheetState = .none

                    // use imperative ShareSheet for automatic share after NFC read
                    ShareSheet.present(
                        data: hexEncode(bytes: backup),
                        filename: "\(tapSigner.identFileNamePrefix())_backup.txt"
                    ) { _ in }
                }

            case let .failure(error):
                if error.isAuthError() {
                    app.sheetState = nil
                    app.alertState = .init(.tapSignerWrongPin(tapSigner: tapSigner, action: .backup))
                } else {
                    app.alertState = .init(
                        .general(
                            title: "Backup Failed!",
                            message: "TapSigner backup failed. Please try again."
                        )
                    )
                }

                await MainActor.run { self.pin = "" }
            }
        }
    }

    private func signAction(_ nfc: TapSignerNFC, _ psbt: Psbt, _ pin: String) {
        Task {
            switch await nfc.sign(psbt: psbt, pin: pin) {
            case let .success(signedPsbt):
                do {
                    let db = Database().unsignedTransactions()
                    let txId = psbt.txId()
                    let record = try db.getTxThrow(txId: txId)
                    let route = RouteFactory()
                        .sendConfirmSignedPsbt(
                            id: record.walletId(),
                            details: record.confirmDetails(),
                            psbt: signedPsbt
                        )

                    await MainActor.run {
                        manager.enteredPin = nil
                        self.pin = ""
                        app.sheetState = .none
                        app.pushRoute(route)
                    }
                } catch {
                    await MainActor.run {
                        app.alertState = .init(
                            .general(
                                title: "Error",
                                message: "Unable to load the pending transaction."
                            )
                        )

                        self.pin = ""
                        app.sheetState = .none
                    }
                }
            case let .failure(error):
                if error.isAuthError() {
                    app.sheetState = nil
                    app.alertState = .init(.tapSignerWrongPin(tapSigner: tapSigner, action: .sign(psbt)))
                } else {
                    app.alertState = .init(
                        .general(
                            title: "Signing Failed!",
                            message: "TapSigner signing failed. Please try again."
                        )
                    )
                    app.sheetState = .none
                }

                await MainActor.run { self.pin = "" }
            }
        }
    }

    var body: some View {
        TapSignerCvcScreen(
            cvc: $pin,
            focus: $isFocused,
            spacing: 40,
            header: TapSignerPinHeader(actionTitle: "Cancel", systemImage: nil, action: cancel),
            description: TapSignerPinDescription(
                title: "Enter TAPSIGNER CVC",
                message: message
            ),
            submitTitle: "Continue",
            errorMessage: errorMessage,
            submitAction: submitPin
        )
        .onAppear(perform: resetPin)
        .onDisappear(perform: clearSensitiveState)
    }

    private func cancel() {
        clearSensitiveState()
        manager.cancel()
        app.sheetState = .none
    }

    private func resetPin() {
        pin = ""
        errorMessage = nil
        isFocused = true
    }

    private func submitPin() {
        guard let inputError = tapSignerCvcInputError(hex: pin) else {
            let nfc = manager.getOrCreateNfc(tapSigner)
            manager.enteredPin = pin

            isFocused = false
            runAction(nfc, pin)
            return
        }

        errorMessage = inputError.errorDescription
    }

    private func clearSensitiveState() {
        pin = ""
        errorMessage = nil
        isFocused = false
    }
}

#Preview {
    TapSignerContainer(
        route: .enterPin(
            tapSigner: tapSignerPreviewNew(preview: true),
            action: .derive
        )
    )
    .environment(AppManager.shared)
    .environment(AuthManager.shared)
}
