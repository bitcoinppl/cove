//
//  TapSignerNewPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import CoveCore
import SwiftUI

struct TapSignerNewPinView: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let args: TapSignerNewPinArgs

    // private
    @State private var newPin = ""
    @State private var errorMessage: String?
    @FocusState private var isFocused

    var body: some View {
        TapSignerCvcScreen(
            cvc: $newPin,
            focus: $isFocused,
            spacing: 40,
            header: TapSignerPinHeader(actionTitle: "Back", action: goBack),
            description: TapSignerPinDescription(
                title: "Create New CVC",
                message: """
                The CVC prevents unauthorized access to your key. Enter 6 to 32 ASCII digits, then keep the CVC safe. You'll need it for signing transactions.
                """
            ),
            submitTitle: "Continue",
            errorMessage: errorMessage,
            submitAction: continueToConfirmation
        )
        .onAppear(perform: resetPin)
        .onDisappear(perform: clearSensitiveState)
    }

    private func goBack() {
        clearSensitiveState()
        manager.popRoute()
    }

    private func resetPin() {
        newPin = ""
        errorMessage = nil
        isFocused = true
    }

    private func continueToConfirmation() {
        guard let inputError = tapSignerCvcInputError(value: newPin) else {
            isFocused = false
            manager.navigate(to: .confirmPin(TapSignerConfirmPinArgs(from: args, newPin: newPin)))
            return
        }

        errorMessage = inputError
    }

    private func clearSensitiveState() {
        newPin = ""
        errorMessage = nil
        isFocused = false
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
