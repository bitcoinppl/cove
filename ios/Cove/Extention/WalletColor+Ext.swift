import SwiftUI

extension WalletColor {
    func toCardColors() -> [Color] {
        switch self {
        case .red:
            return [.red, .red.opacity(0.6)]
        case .blue:
            return [.blue, .blue.opacity(0.6)]
        case .green:
            return [.green, .green.opacity(0.6)]
        case .yellow:
            return [.yellow, .yellow.opacity(0.6)]
        case .orange:
            return [.orange, .orange.opacity(0.6)]
        case .purple:
            return [.purple, .purple.opacity(0.6)]
        case .pink:
            return [.pink, .pink.opacity(0.6)]
        case let .custom(r, g, b):
            let color = customToColor(r: r, g: g, b: b)
            return [color, color.opacity(0.6)]
        }
    }
}

func customToColor(r: UInt8, g: UInt8, b: UInt8) -> Color {
    Color(red: Double(r), green: Double(g), blue: Double(b))
}
