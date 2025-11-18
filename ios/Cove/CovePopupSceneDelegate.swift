//
//  CovePopupSceneDelegate.swift
//  Cove
//
//  Created by Assistant on 11/18/25.
//

import MijickPopups

final class CovePopupSceneDelegate: PopupSceneDelegate {
    override init() {
        super.init()
        configBuilder = { builder in
            builder
                .vertical {
                    $0
                        .enableDragGesture(true)
                        .tapOutsideToDismissPopup(true)
                        .cornerRadius(32)
                }
                .center {
                    $0
                        .tapOutsideToDismissPopup(true)
                }
        }
    }
}
