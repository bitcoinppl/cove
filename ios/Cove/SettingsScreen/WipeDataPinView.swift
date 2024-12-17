//
//  WipeDataPinView.swift
//  Cove
//
//  Created by Praveen Perera on 12/17/24.
//

import SwiftUI

private enum PinState {
    case new
    case confirm(String)
}

struct WipeDataPinView: View {
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
                    title: "Enter Wipe Data PIN",
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
                    title: "Confirm Wipe Data PIN",
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
    WipeDataPinView(onComplete: { _ in }, backAction: {})
        .environment(AuthManager())
}
