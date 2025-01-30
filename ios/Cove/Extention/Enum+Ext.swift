extension Network: CaseIterable & CustomStringConvertible {
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

extension FiatCurrency: CaseIterable & CustomStringConvertible {
    public var description: String {
        "\(self.emoji()) \(self.toString())"
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
