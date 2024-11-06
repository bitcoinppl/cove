//
//  String+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import Foundation

extension String {
    init(_ unit: Unit) {
        self = unitToString(unit: unit)
    }

    init(_ walletAddressType: WalletAddressType) {
        self = walletAddressTypeToString(walletAddressType: walletAddressType)
    }

    init(_ adress: Address) {
        self = adress.string()
    }

    func removingLeadingZeros() -> String {
        guard self != "0" else { return self }
        if self.contains(".") {
            if self.hasSuffix("0") {
                return self.normalizeZero()
            } else {
                return self
            }
        }

        let int = Int(self) ?? 0
        return String(int)
    }

    func normalizeZero() -> String {
        let pattern = "^0+\\.0$"
        if self.range(of: pattern, options: .regularExpression) != nil {
            return "0.0"
        }
        return self
    }
}
