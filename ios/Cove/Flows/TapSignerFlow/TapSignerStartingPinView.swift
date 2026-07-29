//
//  TapSignerStartingPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerStartingPin: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    var chainCode: String? = nil

    // private
    @State private var startingPin: String = ""
    @FocusState private var isFocused

    var body: some View {
        TapSignerPinScreen(
            pin: $startingPin,
            focus: $isFocused,
            spacing: 30,
            header: TapSignerStartingPinHeader(action: goBack),
            description: TapSignerPinDescription(
                title: "Enter Starting PIN",
                message: """
                The starting PIN is the 6 digit numeric PIN found of the back of your TAPSIGNER
                """
            ),
            indicators: TapSignerPinIndicators(pinCount: startingPin.count, focus: $isFocused)
        )
        .onAppear(perform: resetPin)
        .onChange(of: isFocused, keepFocused)
        .onChange(of: startingPin, handlePinChange)
    }

    private func goBack() {
        manager.popRoute()
    }

    private func resetPin() {
        startingPin = ""
        isFocused = true
    }

    private func keepFocused(_: Bool, _: Bool) {
        isFocused = true
    }

    private func handlePinChange(old: String, pin: String) {
        if pin.count == 6 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                manager.navigate(to:
                    .newPin(TapSignerNewPinArgs(
                        tapSigner: tapSigner,
                        startingPin: pin,
                        chainCode: chainCode,
                        action: .setup
                    )))
            }
        }

        if pin.count > 6, old.count < 6 {
            startingPin = old
            return
        }

        if pin.count > 6 {
            startingPin = String(startingPin.prefix(6))
        }
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
