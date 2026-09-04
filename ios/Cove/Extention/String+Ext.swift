//
//  String+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import Foundation

typealias Unit = CoveCore.BitcoinUnit

extension String {
    init(_ unit: Unit) {
        self = unit.description
    }

    init(_ walletAddressType: WalletAddressType) {
        self = walletAddressType.description
    }

    init(_ address: Address) {
        self = address.unformatted()
    }

    init(_ walletType: WalletType) {
        self = walletType.description
    }

    init(_ fingeprint: Fingerprint) {
        self = fingeprint.asUppercase()
    }

    init(_ feeSpeed: FeeSpeed) {
        self = feeSpeed.description
    }

    init(_ network: Network) {
        self = network.description
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
