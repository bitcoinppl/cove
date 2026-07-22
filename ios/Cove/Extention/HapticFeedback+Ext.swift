//
//  HapticFeedback+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 12/3/24.
//

import UIKit

enum AppHaptics {
    static func selectionChanged() {
        let generator = UISelectionFeedbackGenerator()
        generator.selectionChanged()
    }

    static func pinKeyPressed() {
        let generator = UIImpactFeedbackGenerator(style: .light)
        generator.impactOccurred()
    }
}

extension HapticFeedback {
    func trigger() {
        switch self {
        case .progress:
            let generator = UIImpactFeedbackGenerator(style: .light)
            generator.impactOccurred()
        case .success:
            let generator = UINotificationFeedbackGenerator()
            generator.notificationOccurred(.success)
        case .none:
            break
        }
    }
}
