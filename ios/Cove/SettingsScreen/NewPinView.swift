//
//  NewPinView.swift
//  Cove
//
//  Created by Praveen Perera on 12/12/24.
//

import SwiftUI

private enum PinState {
    case new, confirm(String)
}

struct NewPinView: View {
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
                    title: "Enter New PIN",
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
                    title: "Confirm New PIN",
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
    NewPinView(onComplete: { _ in }, backAction: {})
}
