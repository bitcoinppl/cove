//
//  TapSignerConfirmPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import CoveCore
import SwiftUI

struct TapSignerConfirmPinView: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let args: TapSignerConfirmPinArgs

    // private
    @State private var confirmPin = ""
    @State private var errorMessage: String?
    @FocusState private var isFocused

    private func checkPin() {
        if let inputError = tapSignerCvcInputError(value: confirmPin) {
            errorMessage = inputError.errorDescription
            return
        }

        if let inputError = tapSignerCvcInputError(value: args.newPin) {
            errorMessage = inputError.errorDescription
            return
        }

        guard confirmPin == args.newPin else {
            errorMessage = "The CVCs do not match."
            confirmPin = ""
            return
        }

        isFocused = false
        switch args.action {
        case .setup: setupTapSigner()
        case .change: changeTapSignerPin()
        }
    }

    private func setupTapSigner() {
        guard let chainCode = args.chainCode else {
            errorMessage = TapSignerChainCodeInputError.invalidLength.errorDescription
            return
        }

        guard let chainCodeBytes = tapSignerChainCodeBytes(hex: chainCode) else {
            errorMessage = tapSignerChainCodeInputError(hex: chainCode)?.errorDescription
            return
        }

        let nfc = manager.getOrCreateNfc(args.tapSigner)

        Task {
            let response = await nfc.setupTapSigner(
                factoryPin: args.startingPin, newPin: args.newPin, chainCode: chainCodeBytes
            )
            await MainActor.run {
                switch response {
                case let .success(.complete(c)):
                    manager.resetRoute(to: .setupSuccess(args.tapSigner, c))
                case let .success(incomplete):
                    manager.resetRoute(to: .setupRetry(args.tapSigner, incomplete))
                case .failure:
                    // failed to setup but we can continue
                    if let incomplete = nfc.lastResponse()?.setupResponse {
                        return manager.resetRoute(to: .setupRetry(args.tapSigner, incomplete))
                    }

                    // failed to setup and can't continue from a screen, send back to home and ask them to restart the process
                    Log.error("TapSigner setup failed")
                    app.sheetState = .none
                    app.alertState = .init(
                        .tapSignerSetupFailed(
                            message: "TapSigner setup failed. Please try again."
                        )
                    )
                }
            }
        }
    }

    private func changeTapSignerPin() {
        let nfc = manager.getOrCreateNfc(args.tapSigner)
        Task {
            let response = await nfc.changePin(
                currentPin: args.startingPin, newPin: args.newPin
            )
            await MainActor.run {
                switch response {
                case .success:
                    clearSensitiveState()
                    manager.cancel()
                    app.sheetState = .none
                    app.alertState = .init(
                        .general(
                            title: "PIN Changed",
                            message: "Your TAPSIGNER PIN was changed successfully!"
                        )
                    )
                case let .failure(error):
                    if error.isAuthError() { return app.alertState = .init(.tapSignerInvalidAuth) }
                    if error.isNoBackupError() { return app.alertState = .init(.tapSignerNoBackup(tapSigner: args.tapSigner)) }
                    app.alertState = .init(
                        .general(
                            title: "Error",
                            message: "TapSigner PIN change failed. Please try again."
                        )
                    )
                }
            }
        }
    }

    var body: some View {
        TapSignerCvcScreen(
            cvc: $confirmPin,
            focus: $isFocused,
            spacing: 40,
            header: TapSignerPinHeader(actionTitle: "Back", action: goBack),
            description: TapSignerPinDescription(
                title: "Confirm New CVC",
                message: """
                Enter the same CVC again to confirm it. The CVC prevents unauthorized access to your key.
                """
            ),
            submitTitle: "Continue",
            errorMessage: errorMessage,
            submitAction: checkPin
        )
        .onAppear(perform: resetPin)
        .onDisappear(perform: clearSensitiveState)
    }

    private func goBack() {
        clearSensitiveState()
        manager.popRoute()
    }

    private func resetPin() {
        confirmPin = ""
        errorMessage = nil
        isFocused = true
    }

    private func clearSensitiveState() {
        confirmPin = ""
        errorMessage = nil
        isFocused = false
    }
}

#Preview {
    TapSignerContainer(
        route:
        .confirmPin(
            TapSignerConfirmPinArgs(
                tapSigner: tapSignerPreviewNew(preview: true),
                startingPin: "123456",
                newPin: "222222",
                chainCode: nil,
                action: .setup
            )
        )
    )
    .environment(AppManager.shared)
}
