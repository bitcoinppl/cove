//
//  MiddlePopup.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import MijickPopups
import SwiftUI

struct MiddlePopup: CenterPopup {
    /// required
    var state: PopupState

    // optional
    var configure: ((Config) -> Config)?
    var heading: String?
    var message: String?
    var buttonText: String = "OK"
    var onClose: () -> Void = {}
    @State var swipeToDismiss = true

    var body: some View {
        MiddlePopupView(state: state, dismiss: { Task { await dismissLastPopup() } }, heading: heading, message: message, buttonText: buttonText, onClose: onClose)
            .padding(24)
            .gesture(
                DragGesture()
                    .onEnded { gesture in
                        if abs(gesture.translation.width) > 40 || abs(gesture.translation.height) > 40 {
                            if swipeToDismiss {
                                Task { await dismissLastPopup() }
                            }
                        }
                    }
            )
    }

    func configurePopup(config: Config) -> Config {
        if let configure {
            return configure(config)
        }

        return config.tapOutsideToDismissPopup(true)
            .popupHorizontalPadding(30)
            .backgroundColor(.clear)
            .overlayColor(.black.opacity(0.5))
    }
}
