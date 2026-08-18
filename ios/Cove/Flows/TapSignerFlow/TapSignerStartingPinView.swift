//
//  TapSignerStartingPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import CoveCore
import SwiftUI

struct TapSignerStartingPin: View {
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    var chainCode: String? = nil

    // private
    @State private var startingPin = ""
    @State private var errorMessage: String?
    @FocusState private var isFocused

    var body: some View {
        TapSignerCvcScreen(
            cvc: $startingPin,
            focus: $isFocused,
            spacing: 30,
            header: TapSignerStartingPinHeader(action: goBack),
            description: TapSignerPinDescription(
                title: "Enter Factory CVC",
                message: """
                The factory code is the 6 digit ASCII code printed on the back of your TAPSIGNER. Enter it here.
                """
            ),
            submitTitle: "Continue",
            errorMessage: errorMessage,
            submitAction: continueSetup
        )
        .onAppear(perform: resetPin)
        .onDisappear(perform: clearSensitiveState)
    }

    private func resetPin() {
        startingPin = ""
        errorMessage = nil
        isFocused = true
    }

    private func continueSetup() {
        guard let inputError = tapSignerCvcInputError(value: startingPin) else {
            isFocused = false
            manager.navigate(to: .newPin(
                TapSignerNewPinArgs(
                    tapSigner: tapSigner,
                    startingPin: startingPin,
                    chainCode: chainCode,
                    action: .setup
                )
            ))
            return
        }

        errorMessage = inputError.errorDescription
    }

    private func goBack() {
        clearSensitiveState()
        manager.popRoute()
    }

    private func clearSensitiveState() {
        startingPin = ""
        errorMessage = nil
        isFocused = false
    }
}

#Preview {
    TapSignerContainer(route:
        .startingPin(
            tapSigner: tapSignerPreviewNew(preview: true),
            chainCode: nil
        ))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
