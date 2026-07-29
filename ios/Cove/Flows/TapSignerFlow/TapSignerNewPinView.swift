//
//  TapSignerNewPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerNewPinView: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let args: TapSignerNewPinArgs

    // private
    @State private var newPin: String = ""
    @FocusState private var isFocused

    var body: some View {
        TapSignerPinScreen(
            pin: $newPin,
            focus: $isFocused,
            spacing: 40,
            header: TapSignerPinHeader(actionTitle: "Back", action: goBack),
            description: TapSignerPinDescription(
                title: "Create New PIN",
                message: """
                The PIN code is a security feature that prevents unauthorized access to your key. \
                Please back it up and keep it safe. You'll need it for signing transactions.
                """
            ),
            indicators: TapSignerPinIndicators(pinCount: newPin.count, focus: $isFocused)
        )
        .onAppear(perform: resetPin)
        .onChange(of: isFocused, keepFocused)
        .onChange(of: newPin, handlePinChange)
    }

    private func goBack() {
        manager.popRoute()
    }

    private func resetPin() {
        newPin = ""
        isFocused = true
    }

    private func keepFocused(_: Bool, _: Bool) {
        isFocused = true
    }

    private func handlePinChange(old: String, pin: String) {
        if pin.count == 6 {
            manager.navigate(
                to: .confirmPin(TapSignerConfirmPinArgs(from: args, newPin: pin))
            )
            return
        }

        if pin.count > 6, old.count < 6 {
            newPin = old
            return
        }

        if pin.count > 6 {
            newPin = String(args.startingPin.prefix(6))
        }
    }
}

#Preview {
    TapSignerContainer(
        route: .newPin(
            TapSignerNewPinArgs(
                tapSigner: tapSignerPreviewNew(preview: true),
                startingPin: "123456",
                chainCode: nil,
                action: .setup
            )
        )
    )
    .environment(AppManager.shared)
    .environment(AuthManager.shared)
}
