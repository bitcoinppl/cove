//
//  FloaterPopup.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import MijickPopupView
import SwiftUI

struct FloaterPopup: TopPopup {
    // required
    let text: String

    // optional
    let backgroundColor = Color.black
    let textColor = Color.white
    let iconColor = Color.green
    let icon = "checkmark"

    let configure: ((TopPopupConfig) -> TopPopupConfig)? = nil

    func createContent() -> some View {
        FloaterPopupView(text: text, backgroundColor: backgroundColor, textColor: textColor, iconColor: iconColor, icon: icon)
    }

    func configurePopup(popup: TopPopupConfig) -> TopPopupConfig {
        if let configure = configure {
            return configure(popup)
        }

        return popup
            .tapOutsideToDismiss(true)
            .horizontalPadding(30)
            .backgroundColour(.clear)
    }
}
