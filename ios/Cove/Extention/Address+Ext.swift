//
//  Address+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 11/7/24.
//

extension Address {
    static func isValid(_ address: String, network: Network? = nil) -> Bool {
        if address.isEmpty { return false }

        if let network = network {
            return addressIsValidForNetwork(address: address, network: network)
        }

        return addressIsValid(address: address)
    }
}
