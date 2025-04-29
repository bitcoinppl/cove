//
//  Error+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 12/17/24.
//

extension AuthManagerError {
    var describe: String {
        describeAuthManagerError(error: self)
    }
}

extension MultiFormatError {
    var describe: String {
        describeMultiFormatError(error: self)
    }
}

extension WalletError {
    var describe: String {
        describeWalletError(error: self)
    }
}

extension TransportError {
    init(code: Int, message: String) {
        self = createTransportErrorFromCode(code: UInt16(code), message: message)
    }
}

extension TapSignerReaderError {
    var describe: String {
        describeTapSignerReaderError(error: self)
    }

    var isAuthError: Bool {
        tapSignerErrorIsAuthError(error: self)
    }

    var isNoBackupError: Bool {
        tapSignerErrorIsNoBackupError(error: self)
    }
}

extension WalletManagerError {
    var describe: String {
        describeWalletManagerError(error: self)
    }
}

extension SendFlowFiatOnChangeError {
    var describe: String {
        describeSendFlowFiatOnChangeError(error: self)
    }
}

