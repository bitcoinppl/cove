import Foundation

extension Network: @retroactive CaseIterable {}
extension Network: SettingsEnum {
    public static var allCases: [Network] {
        allNetworks()
    }
}

extension FiatCurrency: @retroactive CaseIterable {}
extension FiatCurrency: SettingsEnum {
    public static var allCases: [FiatCurrency] {
        allFiatCurrencies()
    }

    var displayName: String {
        "\(emojiString()) \(description)"
    }
}

extension ColorSchemeSelection: @retroactive CaseIterable {}
extension ColorSchemeSelection: @retroactive CustomStringConvertible {}
extension ColorSchemeSelection: SettingsEnum {
    public var description: String {
        capitalizedString()
    }

    public static var allCases: [ColorSchemeSelection] {
        allColorSchemes()
    }
}
