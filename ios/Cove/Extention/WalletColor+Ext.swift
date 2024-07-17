import SwiftUI

extension WalletColor {
    func all() -> [WalletColor] {
        [
            .red,
            .blue,
            .green,
            .yellow,
            .orange,
            .purple,
            .pink,
        ]
    }

    func toCardColors() -> [Color] {
        switch self {
        case .red:
            return [.red, .red.opacity(0.99)]
        case .blue:
            return [.blue, .blue.opacity(0.99)]
        case .green:
            return [.green, .green.opacity(0.99)]
        case .yellow:
            return [.yellow, .yellow.opacity(0.99)]
        case .orange:
            return [.orange, .orange.opacity(0.99)]
        case .purple:
            return [.purple, .purple.opacity(0.99)]
        case .pink:
            return [.pink, .pink.opacity(0.99)]
        case let .custom(r, g, b):
            let color = customToColor(r: r, g: g, b: b)
            return [color, color.opacity(0.99)]
        }
    }

    func toColor() -> Color {
        switch self {
        case .red:
            return .red
        case .blue:
            return .blue
        case .green:
            return .green
        case .yellow:
            return .yellow
        case .orange:
            return .orange
        case .purple:
            return .purple
        case .pink:
            return .pink
        case let .custom(r, g, b):
            return Color(red: Double(r) / 255, green: Double(g) / 255, blue: Double(b) / 255)
        }
    }
}

extension Color {
    func toWalletColor() -> WalletColor {
        switch self {
        case .red:
            return .red
        case .blue:
            return .blue
        case .green:
            return .green
        case .yellow:
            return .yellow
        case .orange:
            return .orange
        case .purple:
            return .purple
        case .pink:
            return .pink
        case let color:
            let (red, green, blue, _) = color.getRGB()
            return .custom(r: UInt8(red), g: UInt8(green), b: UInt8(blue))
        }
    }
}

func customToColor(r: UInt8, g: UInt8, b: UInt8) -> Color {
    Color(red: Double(r), green: Double(g), blue: Double(b))
}
