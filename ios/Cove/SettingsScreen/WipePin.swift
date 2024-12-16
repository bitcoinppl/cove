//
//  WipePin.swift
//  Cove
//
//  Created by Praveen Perera on 12/15/24.
//

import SwiftUI

private enum PinState {
    case pin, new, confirm(String)
}

struct WipePin: View {
    @Environment(AppManager.self) var app

    /// args
    var onComplete: (String) -> Void
    var backAction: () -> Void

    /// private
    @State private var pinState: PinState = .new

    var body: some View {
        Group {
            switch pinState {
            case .pin:
                NumberPadPinView(
                    title: "Enter Current PIN",
                    isPinCorrect: app.checkPin,
                    showPin: false,
                    backAction: backAction,
                    onUnlock: { _ in
                        withAnimation {
                            pinState = .new
                        }
                    }
                )
            case .new:
                NumberPadPinView(
                    title: "Enter Wipe Me PIN",
                    isPinCorrect: { _ in true },
                    showPin: true,
                    backAction: backAction,
                    onUnlock: { enteredPin in
                        withAnimation {
                            pinState = .confirm(enteredPin)
                        }
                    }
                )
            case let .confirm(pinToConfirm):
                NumberPadPinView(
                    title: "Confirm Wipe Me PIN",
                    isPinCorrect: { $0 == pinToConfirm },
                    showPin: true,
                    backAction: backAction,
                    onUnlock: onComplete
                )
            }
        }
    }
}

#Preview {
    WipePin(onComplete: { _ in }, backAction: {})
        .environment(AppManager())
}
