//
//  Error+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 12/17/24.
//

extension TransportError {
    init(code: Int, message: String) {
        self = createTransportErrorFromCode(code: UInt16(code), message: message)
    }
}

extension TapSignerReaderError {
    var isAuthError: Bool {
        tapSignerErrorIsAuthError(error: self)
    }

    var isNoBackupError: Bool {
        tapSignerErrorIsNoBackupError(error: self)
    }
}
