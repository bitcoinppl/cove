//
//  TapSignerConfirmPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerConfirmPinView: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let args: TapSignerConfirmPinArgs

    // private
    @State private var confirmPin: String = ""
    @State private var animateField: Bool = false
    @FocusState private var isFocused

    var chainCodeBytes: Data? {
        guard let chainCode = args.chainCode else { return nil }
        return hexDecode(hex: chainCode)
    }

    func checkPin() {
        if confirmPin != args.newPin {
            animateField.toggle()
            confirmPin = ""
            return
        }

        switch args.action {
        case .setup: setupTapSigner()
        case .change: changeTapSignerPin()
        }
    }

    func setupTapSigner() {
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

    func changeTapSignerPin() {
        let nfc = manager.getOrCreateNfc(args.tapSigner)
        Task {
            let response = await nfc.changePin(
                currentPin: args.startingPin, newPin: args.newPin
            )
            switch response {
            case .success:
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

    var body: some View {
        TapSignerPinScreen(
            pin: $confirmPin,
            focus: $isFocused,
            spacing: 40,
            header: TapSignerPinHeader(actionTitle: "Back", action: goBack),
            description: TapSignerPinDescription(
                title: "Confirm New PIN",
                message: """
                The PIN code is a security feature that prevents unauthorized access to your key. \
                Please back it up and keep it safe. You'll need it for signing transactions.
                """
            ),
            indicators: TapSignerShakingPinIndicators(
                pinCount: confirmPin.count,
                animateField: animateField,
                focus: $isFocused
            )
        )
        .onAppear(perform: resetPin)
        .onChange(of: isFocused, keepFocused)
        .onChange(of: confirmPin, handlePinChange)
    }

    private func goBack() {
        manager.popRoute()
    }

    private func resetPin() {
        confirmPin = ""
        isFocused = true
    }

    private func keepFocused(_: Bool, _: Bool) {
        isFocused = true
    }

    private func handlePinChange(old: String, pin: String) {
        if pin.count == 6 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                checkPin()
            }
        }

        if pin.count > 6, old.count < 6 {
            confirmPin = old
            return
        }

        if pin.count > 6 {
            confirmPin = String(args.startingPin.prefix(6))
        }
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
