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
        default:
            let color = self.toColor()
            return [color, color.opacity(0.99)]
        }
    }

    func toColor() -> Color {
        switch self {
        case .red:
            .red
        case .blue:
            .blue
        case .green:
            .green
        case .yellow:
            .yellow
        case .orange:
            .orange
        case .purple:
            .purple
        case .pink:
            .pink
        case .coolGray:
            .coolGray
        case .wPastelTeal:
            .pastelTeal
        case .wLightPastelYellow:
            .lightPastelYellow
        case .wLightMint:
            .lightMint
        case .wPastelBlue:
            .pastelBlue
        case .wPastelNavy:
            .pastelNavy
        case .wPastelRed:
            .pastelRed
        case .wPastelYellow:
            .pastelYellow
        case .wAlmostGray:
            .almostGray
        case .wAlmostWhite:
            .almostWhite
        case .wBeige:
            .beige
        case let .custom(r, g, b):
            Color(red: Double(r) / 255, green: Double(g) / 255, blue: Double(b) / 255)
        }
    }
}

extension FfiColor {
    func toColor() -> Color {
        switch self {
        case let .red(opacity):
            .red.addOpacity(opacity)
        case let .blue(opacity):
            .blue.addOpacity(opacity)
        case let .green(opacity):
            .green.addOpacity(opacity)
        case let .yellow(opacity):
            .yellow.addOpacity(opacity)
        case let .orange(opacity):
            .orange.addOpacity(opacity)
        case let .purple(opacity):
            .purple.addOpacity(opacity)
        case let .pink(opacity):
            .pink.addOpacity(opacity)
        case let .white(opacity):
            .white.addOpacity(opacity)
        case let .black(opacity):
            .black.addOpacity(opacity)
        case let .gray(opacity):
            .gray.addOpacity(opacity)
        case let .coolGray(opacity):
            .coolGray.addOpacity(opacity)
        case let .custom(rgb, opacity):
            customToColor(r: rgb.r, g: rgb.g, b: rgb.b).addOpacity(opacity)
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

    func addOpacity(_ opacity: FfiOpacity) -> Color {
        if opacity == 100 {
            return self
        }

        return self.opacity(Double(opacity) / 100)
    }
}

func customToColor(r: UInt8, g: UInt8, b: UInt8) -> Color {
    Color(red: Double(r), green: Double(g), blue: Double(b))
}
