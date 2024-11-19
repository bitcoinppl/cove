//
//  Format.swift
//  Cove
//
//  Created by Praveen Perera on 9/16/24.
//

import Foundation

struct ThousandsFormatter<T: Numeric & LosslessStringConvertible> {
    let number: T

    init(_ number: T) {
        self.number = number
    }

    func fmt() -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.groupingSeparator = ","
        formatter.groupingSize = 3

        return formatter.string(from: NSNumber(value: Double(String(number))!)) ?? String(number)
    }

    func fmtWithUnit() -> String {
        fmt() + " SATS"
    }
}

struct FiatFormatter<T: Numeric & LosslessStringConvertible> {
    let number: T

    init(_ number: T) {
        self.number = number
    }

    func fmt() -> String {
        let f = NumberFormatter()
        f.numberStyle = .currency
        f.minimumFractionDigits = 2
        f.maximumFractionDigits = 2

        return f.string(from: NSNumber(value: Double(String(number))!)) ?? String(number)
    }

    func fmtWithUnit() -> String {
        fmt() + " USD"
    }
}
