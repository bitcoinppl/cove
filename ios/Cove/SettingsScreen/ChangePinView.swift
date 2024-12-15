//
//  ChangePinView.swift
//  Cove
//
//  Created by Praveen Perera on 12/12/24.
//

import SwiftUI

private enum PinState {
    case current, new, confirm(String)
}

struct ChangePinView: View {
    /// args
    var isPinCorrect: (String) -> Bool
    var backAction: () -> Void
    var onComplete: (String) -> Void

    /// private
    @State private var pinState: PinState = .current

    var body: some View {
        Group {
            switch pinState {
            case .current:
                NumberPadPinView(
                    title: "Enter Current PIN",
                    isPinCorrect: isPinCorrect,
                    backAction: backAction,
                    onUnlock: { _ in
                        withAnimation {
                            pinState = .new
                        }
                    }
                )

            case .new:
                NumberPadPinView(
                    title: "Enter new PIN",
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
    ChangePinView(
        isPinCorrect: { pin in pin == "111111" },
        backAction: {},
        onComplete: { _ in }
    )
}
