//
//  SignedTransactionOrPsbt+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 1/22/26.
//

import CoveCore
import Foundation

extension SignedTransactionOrPsbt {
    static func tryFromNfcMessage(nfcMessage: NfcMessage) throws -> SignedTransactionOrPsbt {
        try signedTransactionOrPsbtTryFromNfcMessage(nfcMessage: nfcMessage)
    }

    static func tryParse(input: String) throws -> SignedTransactionOrPsbt {
        try signedTransactionOrPsbtTryParse(input: input)
    }

    static func tryFromBytes(data: Data) throws -> SignedTransactionOrPsbt {
        try signedTransactionOrPsbtTryFromBytes(data: data)
    }
}
