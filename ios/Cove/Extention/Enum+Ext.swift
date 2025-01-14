extension Network {
    func toString() -> String {
        networkToString(network: self)
    }
}

extension FiatCurrency {
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
