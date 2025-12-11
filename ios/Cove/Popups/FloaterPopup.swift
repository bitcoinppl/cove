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
    var backgroundColor: Color = .black
    var textColor: Color = .white
    var iconColor: Color = .green
    var icon: String = "checkmark"

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
            .overlayColor(.black.opacity(0.001))
            .backgroundColor(.clear)
    }
}
