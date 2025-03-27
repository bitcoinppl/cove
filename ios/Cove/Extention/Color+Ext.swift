//
//  Color+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 6/21/24.
//

import Foundation
import SwiftUI

struct LabeledColor: Identifiable {
    var id: String { name }

    var color: Color
    var name: String

    static var allColors: [LabeledColor] {
        Color.allColors.map { LabeledColor(color: $0.0, name: $0.1) }
    }
}

extension Color {
    static var coolGray: Color {
        Color(hue: 0.61, saturation: 0.04, brightness: 0.83, opacity: 1.00)
    }

    static var lightGreen: Color {
        Color(red: 0.463, green: 0.898, blue: 0.584) // #76e595
    }

    static var background: Color {
        Color(UIColor.systemBackground)
    }

    static var secondaryBackground: Color {
        Color(UIColor.secondarySystemBackground)
    }

    static var listBackground: Color {
        Color(UIColor.systemGroupedBackground)
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

    func toHexString(colorScheme: ColorScheme) -> String {
        let resolvedColor = UIColor(self).resolvedColor(
            with:
            UITraitCollection(userInterfaceStyle: colorScheme == .dark ? .dark : .light))

        // Get the RGB values regardless of color space
        var r: CGFloat = 0
        var g: CGFloat = 0
        var b: CGFloat = 0
        var a: CGFloat = 0

        resolvedColor.getRed(&r, green: &g, blue: &b, alpha: &a)

        return String(
            format: "#%02lX%02lX%02lX",
            lround(r * 255),
            lround(g * 255),
            lround(b * 255)
        )
    }

    var hasDarkVariant: Bool {
        let light = UIColor(self)
            .resolvedColor(with: UITraitCollection(userInterfaceStyle: .light))

        let dark = UIColor(self)
            .resolvedColor(with: UITraitCollection(userInterfaceStyle: .dark))

        return light != dark
    }

    // MARK: - Text Colors

    static let lightText = Color(UIColor.lightText)
    static let darkText = Color(UIColor.darkText)
    static let placeholderText = Color(UIColor.placeholderText)

    // MARK: - Label Colors

    static let label = Color(UIColor.label)
    static let secondaryLabel = Color(UIColor.secondaryLabel)
    static let tertiaryLabel = Color(UIColor.tertiaryLabel)
    static let quaternaryLabel = Color(UIColor.quaternaryLabel)

    // MARK: - Background Colors

    static let systemBackground = Color(UIColor.systemBackground)
    static let secondarySystemBackground = Color(UIColor.secondarySystemBackground)
    static let tertiarySystemBackground = Color(UIColor.tertiarySystemBackground)

    // MARK: - Fill Colors

    static let systemFill = Color(UIColor.systemFill)
    static let secondarySystemFill = Color(UIColor.secondarySystemFill)
    static let tertiarySystemFill = Color(UIColor.tertiarySystemFill)
    static let quaternarySystemFill = Color(UIColor.quaternarySystemFill)

    // MARK: - Grouped Background Colors

    static let systemGroupedBackground = Color(UIColor.systemGroupedBackground)
    static let secondarySystemGroupedBackground = Color(UIColor.secondarySystemGroupedBackground)
    static let tertiarySystemGroupedBackground = Color(UIColor.tertiarySystemGroupedBackground)

    // MARK: - Gray Colors

    static let systemGray = Color(UIColor.systemGray)
    static let systemGray2 = Color(UIColor.systemGray2)
    static let systemGray3 = Color(UIColor.systemGray3)
    static let systemGray4 = Color(UIColor.systemGray4)
    static let systemGray5 = Color(UIColor.systemGray5)
    static let systemGray6 = Color(UIColor.systemGray6)

    // MARK: - Other Colors

    static let separator = Color(UIColor.separator)
    static let opaqueSeparator = Color(UIColor.opaqueSeparator)
    static let link = Color(UIColor.link)

    // MARK: System Colors

    static let systemBlue = Color(UIColor.systemBlue)
    static let systemPurple = Color(UIColor.systemPurple)
    static let systemGreen = Color(UIColor.systemGreen)
    static let systemYellow = Color(UIColor.systemYellow)
    static let systemOrange = Color(UIColor.systemOrange)
    static let systemPink = Color(UIColor.systemPink)
    static let systemRed = Color(UIColor.systemRed)
    static let systemTeal = Color(UIColor.systemTeal)
    static let systemIndigo = Color(UIColor.systemIndigo)

    fileprivate static let allColors: [(Self, String)] = [
        // MARK: CustomColors

        (almostGray, "almostGray"),
        (almostWhite, "almostWhite"),
        (beige, "beige"),
        (btnPrimary, "btnPrimary"),
        (coolGray, "coolGray"),
        (coveBg, "coveBg"),
        (coveLightGray, "coveLightGray"),
        (lightGreen, "lightGreen"),
        (midnightBlue, "midnightBlue"),
        (midnightBtn, "midnightBtn"),
        (lightMint, "lightMint"),
        (lightPastelYellow, "lightPastelYellow"),
        (pastelBlue, "pastelBlue"),
        (pastelNavy, "pastelNavy"),
        (pastelRed, "pastelRed"),
        (pastelTeal, "pastelTeal"),
        (pastelYellow, "pastelYellow"),

        // SwiftUI Colors
        (white, "white"),
        (black, "black"),
        (gray, "gray"),
        (red, "red"),
        (orange, "orange"),
        (yellow, "yellow"),
        (green, "green"),
        (blue, "blue"),
        (purple, "purple"),
        (pink, "pink"),
        (secondary, "secondary"),
        (primary, "primary"),
        (accentColor, "accentColor"),

        // Text Colors
        (lightText, "lightText"),
        (darkText, "darkText"),
        (placeholderText, "placeholderText"),

        // Background Colors
        (systemBackground, "systemBackground"),
        (secondarySystemBackground, "secondarySystemBackground"),
        (tertiarySystemBackground, "tertiarySystemBackground"),

        // MARK: System Colors

        (systemBlue, "systemBlue"),
        (systemPurple, "systemPurple"),
        (systemGreen, "systemGreen"),
        (systemYellow, "systemYellow"),
        (systemOrange, "systemOrange"),
        (systemPink, "systemPink"),
        (systemRed, "systemRed"),
        (systemTeal, "systemTeal"),
        (systemIndigo, "systemIndigo"),

        // Gray Colors
        (systemGray, "systemGray"),
        (systemGray2, "systemGray2"),
        (systemGray3, "systemGray3"),
        (systemGray4, "systemGray4"),
        (systemGray5, "systemGray5"),
        (systemGray6, "systemGray6"),

        // Label Colors
        (label, "label"),
        (secondaryLabel, "secondaryLabel"),
        (tertiaryLabel, "tertiaryLabel"),
        (quaternaryLabel, "quaternaryLabel"),

        // Fill Colors
        (systemFill, "systemFill"),
        (secondarySystemFill, "secondarySystemFill"),
        (tertiarySystemFill, "tertiarySystemFill"),
        (quaternarySystemFill, "quaternarySystemFill"),

        // Grouped Background Colors
        (systemGroupedBackground, "systemGroupedBackground"),
        (secondarySystemGroupedBackground, "secondarySystemGroupedBackground"),
        (tertiarySystemGroupedBackground, "tertiarySystemGroupedBackground"),

        // Other Colors
        (separator, "separator"),
        (opaqueSeparator, "opaqueSeparator"),
        (link, "link"),
    ]
}

extension WalletColor {
    var defaultColors: [Self] {
        defaultWalletColors()
    }
}
