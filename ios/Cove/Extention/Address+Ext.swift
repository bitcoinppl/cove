//
//  Address+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 11/7/24.
//

import CoveCore

extension Address {
    static func checkValid(_ address: String, network: Network? = nil) -> Result<Void, AddressError> {
        if address.isEmpty { return .failure(AddressError.EmptyAddress) }

        if let network {
            return Result { try addressIsValidForNetwork(address: address, network: network) }
                .mapError { $0 as! AddressError }
        }

        let network = Database().globalConfig().selectedNetwork()
        return Result { try addressIsValid(address: address, network: network) }
            .mapError { $0 as! AddressError }
    }

    static func isValid(_ address: String, network: Network? = nil) -> Bool {
        Address.checkValid(address, network: network).isSuccess()
    }
}
