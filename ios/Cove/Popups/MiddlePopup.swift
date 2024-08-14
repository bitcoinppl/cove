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
    var configure: ((CentrePopupConfig) -> CentrePopupConfig)?

    // optional
    var heading: String?
    var message: String?
    var buttonText: String = "OK"
    var onClose: () -> Void = {}

    func createContent() -> some View {
        MiddlePopupView(state: state, heading: heading, message: message, buttonText: buttonText, onClose: onClose)
            .padding(24)
    }

    func configurePopup(popup: CentrePopupConfig) -> CentrePopupConfig {
        if let configure = configure {
            return configure(popup)
        }

        return popup.tapOutsideToDismiss(true)
            .horizontalPadding(30)
    }
}
