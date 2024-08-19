//
//  MiddlePopup.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import MijickPopupView
import SwiftUI

struct MiddlePopup: CentrePopup {
    // required
    var state: PopupState

    // optional
    var configure: ((CentrePopupConfig) -> CentrePopupConfig)?
    var heading: String?
    var message: String?
    var buttonText: String = "OK"
    var onClose: () -> Void = {}
    @State var swipeToDismiss = true

    func createContent() -> some View {
        MiddlePopupView(state: state, dismiss: dismiss, heading: heading, message: message, buttonText: buttonText, onClose: onClose)
            .padding(24)
            .gesture(
                DragGesture()
                    .onEnded { gesture in
                        if abs(gesture.translation.width) > 40 || abs(gesture.translation.height) > 40 {
                            if swipeToDismiss {
                                dismiss()
                            }
                        }
                    }
            )
    }

    func configurePopup(popup: CentrePopupConfig) -> CentrePopupConfig {
        if let configure = configure {
            return configure(popup)
        }

        return popup.tapOutsideToDismiss(true)
            .horizontalPadding(30)
    }
}
