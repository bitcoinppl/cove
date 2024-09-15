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

    init(_ color: WalletColor) {
        self = color.toColor()
    }

    init(hex: String) {
        let hex = hex.trimmingCharacters(in: CharacterSet.alphanumerics.inverted)
        var int: UInt64 = 0
        Scanner(string: hex).scanHexInt64(&int)
        let a, r, g, b: UInt64
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

    func getRGB() -> (red: Int, green: Int, blue: Int, opacity: Double) {
        let cgColor = UIColor(self).cgColor
        let components = cgColor.components ?? []

        return (
            red: Int(components[0] * 255),
            green: Int(components[1] * 255),
            blue: Int(components[2] * 255),
            opacity: cgColor.alpha
        )
    }
}
