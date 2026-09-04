//
//  Color+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 6/21/24.
//

import Foundation
import SwiftUI

extension Color {
    static var coolGray: Color {
        Color(hue: 0.61, saturation: 0.04, brightness: 0.83, opacity: 1.00)
    }

    static var lightGreen: Color {
        Color(red: 0.463, green: 0.898, blue: 0.584) // #76e595
    }

    // MARK: - Button Gradient Colors

    static var btnGradientLight: Color {
        Color(red: 0.2, green: 0.4, blue: 1.0) // #3366FF
    }

    static var btnGradientDark: Color {
        Color(red: 0.1, green: 0.5, blue: 1.0) // #1A80FF
    }

    static var background: Color {
        Color(UIColor.systemBackground)
    }

    init(_ color: WalletColor) {
        self = color.toColor()
    }

    init(_ color: FfiColor) {
        self = color.toColor()
    }

    init(hex: String) {
        let hex = hex.trimmingCharacters(in: CharacterSet.alphanumerics.inverted)
        var int: UInt64 = 0
        Scanner(string: hex).scanHexInt64(&int)
        let a: UInt64
        let r: UInt64
        let g: UInt64
        let b: UInt64
        switch hex.count {
        case 3: // RGB (12-bit)
            (a, r, g, b) = (255, (int >> 8) * 17, (int >> 4 & 0xF) * 17, (int & 0xF) * 17)
        case 6: // RGB (24-bit)
            (a, r, g, b) = (255, int >> 16, int >> 8 & 0xFF, int & 0xFF)
        case 8: // ARGB (32-bit)
            (a, r, g, b) = (int >> 24, int >> 16 & 0xFF, int >> 8 & 0xFF, int & 0xFF)
        default:
            (a, r, g, b) = (1, 1, 1, 0)
        }

        self.init(
            .sRGB,
            red: Double(r) / 255,
            green: Double(g) / 255,
            blue: Double(b) / 255,
            opacity: Double(a) / 255
        )
    }

    // MARK: - Label Colors

    static let label = Color(UIColor.label)
    static let secondaryLabel = Color(UIColor.secondaryLabel)
    static let tertiaryLabel = Color(UIColor.tertiaryLabel)

    // MARK: - Background Colors

    static let systemBackground = Color(UIColor.systemBackground)
    static let secondarySystemBackground = Color(UIColor.secondarySystemBackground)
    static let systemGroupedBackground = Color(UIColor.systemGroupedBackground)
    static let secondarySystemGroupedBackground = Color(UIColor.secondarySystemGroupedBackground)

    // MARK: - Gray Colors

    static let systemGray4 = Color(UIColor.systemGray4)
    static let systemGray5 = Color(UIColor.systemGray5)
    static let systemGray6 = Color(UIColor.systemGray6)

    // MARK: - Other Colors

    static let link = Color(UIColor.link)
    static let systemRed = Color(UIColor.systemRed)

    // MARK: - Semantic Status Colors

    static let statusSuccess = Color(UIColor.systemGreen)
    static let statusWarning = Color(UIColor.systemOrange)
    static let statusInfo = Color(UIColor.systemBlue)
    static let statusError = Color(UIColor.systemRed)
}
