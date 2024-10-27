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
}
