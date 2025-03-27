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

extension MultiFormatError {
    var describe: String {
        displayMultiFormatError(error: self)
    }
}

extension WalletError {
    var describe: String {
        displayWalletError(error: self)
    }
}

extension TransportError {
    init(code: Int, message: String) {
        self = createTransportErrorFromCode(code: UInt16(code), message: message)
    }
}

extension TapSignerReaderError {
    var describe: String {
        displayTapSignerReaderError(error: self)
    }
}
