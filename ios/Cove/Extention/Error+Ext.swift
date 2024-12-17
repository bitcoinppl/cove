//
//  Error+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 12/17/24.
//

extension AuthManagerError {
    var describe: String {
        authManagerErrorToString(error: self)
    }
}
