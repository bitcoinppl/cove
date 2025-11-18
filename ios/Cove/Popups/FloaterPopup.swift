//
//  FloaterPopup.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import MijickPopups
import SwiftUI

struct FloaterPopup: TopPopup {
    // required
    let text: String

    // optional
    let backgroundColor = Color.black
    let textColor = Color.white
    let iconColor = Color.green
    let icon = "checkmark"

    var body: some View {
        FloaterPopupView(
            text: text, backgroundColor: backgroundColor, textColor: textColor,
            iconColor: iconColor, icon: icon
        )
    }

    func configurePopup(config: TopPopupConfig) -> TopPopupConfig {
        config
            .tapOutsideToDismissPopup(true)
            .popupHorizontalPadding(30)
            .overlayColor(.clear)
            .backgroundColor(.clear)
    }
}
