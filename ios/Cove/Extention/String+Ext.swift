//
//  String+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import Foundation

typealias Unit = CoveCore.Unit

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

    init(_ walletType: WalletType) {
        self = walletTypeToString(walletType: walletType)
    }

    init(_ fingeprint: Fingerprint) {
        self = fingeprint.asUppercase()
    }

    init(_ feeSpeed: FeeSpeed) {
        self = feeSpeedToString(feeSpeed: feeSpeed)
    }

    init(_ network: Network) {
        self = network.toString()
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

    func addressSpacedOut() -> String {
        addressStringSpacedOut(address: self)
    }

    func padLeft(with: String, toLength: Int) -> String {
        if count >= toLength { return self }

        let padding = String(repeating: with, count: toLength - count)
        return padding + self
    }
}

extension String? {
    init(_ fingeprint: Fingerprint?) {
        if let fingeprint { self = String(fingeprint) } else { self = .none }
    }
}
