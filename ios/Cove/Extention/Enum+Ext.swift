import Foundation
import SwiftUI

extension Network: @retroactive CaseIterable {}
extension Network: @retroactive CustomStringConvertible {}
extension Network: SettingsEnum {
    public var description: String {
        toString()
    }

    public static var allCases: [Network] {
        allNetworks()
    }

    func toString() -> String {
        networkToString(network: self)
    }
}

extension FiatCurrency: @retroactive CaseIterable {}
extension FiatCurrency: @retroactive CustomStringConvertible {}
extension FiatCurrency: SettingsEnum {
    public var description: String {
        "\(emoji()) \(toString())"
    }

    public static var allCases: [FiatCurrency] {
        allFiatCurrencies()
    }

    func toString() -> String {
        fiatCurrencyToString(fiatCurrency: self)
    }

    func symbol() -> String {
        fiatCurrencySymbol(fiatCurrency: self)
    }

    func emoji() -> String {
        fiatCurrencyEmoji(fiatCurrency: self)
    }

    func suffix() -> String {
        fiatCurrencySuffix(fiatCurrency: self)
    }
}

extension ColorSchemeSelection: @retroactive CaseIterable {}
extension ColorSchemeSelection: @retroactive CustomStringConvertible {}
extension ColorSchemeSelection: SettingsEnum {
    public var description: String {
        colorSchemeSelectionCapitalizedString(colorScheme: self)
    }

//    var symbol: String {
//        switch self {
//        case .light: return "sun.max.fill"
//        case .dark: return "moon.stars.fill"
//        case .system: return "circle.lefthalf.fill"
//        }
//    }

    public static var allCases: [ColorSchemeSelection] {
        allColorSchemes()
    }
}
