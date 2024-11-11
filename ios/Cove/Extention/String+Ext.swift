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

    init(_ address: Address) {
        self = address.string()
    }

    init(_ fingeprint: Fingerprint) {
        self = fingeprint.asUppercase()
    }

    init(_ feeSpeed: FeeSpeed) {
        self = feeSpeed.toString()
    }

    func removingLeadingZeros() -> String {
        guard self != "0" else { return self }
        if contains(".") {
            if hasSuffix("0") {
                return normalizeZero()
            } else {
                return self
            }
        }

        let int = Int(self) ?? 0
        return String(int)
    }

    func normalizeZero() -> String {
        let pattern = "^0+\\.0$"
        if range(of: pattern, options: .regularExpression) != nil {
            return "0.0"
        }
        return self
    }
}

extension Optional where Wrapped == String {
    init(_ fingeprint: Fingerprint?) {
        if let fingeprint { self = String(fingeprint) } else { self = .none }
    }
}
