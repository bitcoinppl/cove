//
//  Error+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 12/17/24.
//

extension TransportError {
    init(code: Int, message: String) {
        self = transportErrorFromCode(code: UInt16(code), message: message)
    }
}
