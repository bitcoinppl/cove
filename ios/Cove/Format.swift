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
}
