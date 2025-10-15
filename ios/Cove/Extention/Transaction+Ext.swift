//
//  Transaction+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 8/11/24.
//

import Foundation

extension Transaction: @retroactive Identifiable {
    public var id: TxId {
        switch self {
        case let .confirmed(confirmedTransaction):
            confirmedTransaction.id()
        case let .unconfirmed(unconfirmedTransaction):
            unconfirmedTransaction.id()
        }
    }

    func sentAndReceived() -> SentAndReceived {
        switch self {
        case let .confirmed(transaction):
            transaction.sentAndReceived()
        case let .unconfirmed(transaction):
            transaction.sentAndReceived()
        }
    }
}
