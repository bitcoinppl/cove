//
//  DecoyPinView.swift
//  Cove
//
//  Created by Praveen Perera on 01/28/24.
//

import SwiftUI

private enum PinState {
    case new
    case confirm(String)
}

struct DecoyPinView: View {
    /// args
    var onComplete: (String) -> Void
    var backAction: () -> Void

    /// private
    @State private var pinState: PinState = .new

    var body: some View {
        Group {
            switch pinState {
            case .new:
                NumberPadPinView(
                    title: "Enter Decoy PIN",
                    isPinCorrect: { _ in true },
                    showPin: false,
                    backAction: backAction,
                    onUnlock: { enteredPin in
                        withAnimation {
                            pinState = .confirm(enteredPin)
                        }
                    }
                )
            case let .confirm(pinToConfirm):
                NumberPadPinView(
                    title: "Confirm Decoy PIN",
                    isPinCorrect: { $0 == pinToConfirm },
                    showPin: false,
                    backAction: backAction,
                    onUnlock: onComplete
                )
            }
        }
    }
}

#Preview {
    DecoyPinView(onComplete: { _ in }, backAction: {})
        .environment(AuthManager.shared)
}
