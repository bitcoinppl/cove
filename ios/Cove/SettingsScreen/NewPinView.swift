//
//  NewPinView.swift
//  Cove
//
//  Created by Praveen Perera on 12/12/24.
//

import SwiftUI

enum PinState {
    case new, confirm(String)
}

struct NewPinView: View {
    /// args
    var onComplete: (String) -> Void = { _ in }

    /// private
    @State private var pin = ""
    @State private var pinState: PinState = .new

    var body: some View {
        Group {
            switch pinState {
            case .new:
                NumberPadPinView(
                    pin: $pin,
                    isUnlocked: Binding.constant(false),
                    isPinCorrect: { _ in true },
                    onUnlock: { enteredPin in
                        withAnimation {
                            pinState = .confirm(enteredPin)
                            pin = ""
                        }
                    }
                )
            case .confirm(let pinToConfirm):
                NumberPadPinView(
                    title: "Confirm Pin",
                    pin: $pin,
                    isUnlocked: .constant(false),
                    isPinCorrect: {
                        $0 == pinToConfirm
                    },
                    onUnlock: { _ in
                        onComplete(pinToConfirm)
                    }
                )
            }
        }
    }
}

#Preview {
    NewPinView()
}
