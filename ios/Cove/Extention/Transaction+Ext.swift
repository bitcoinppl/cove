//
//  Transaction+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 8/11/24.
//

import Foundation

extension Transaction: Identifiable {
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

extension TxId: Hashable, Equatable {
    public static func == (lhs: TxId, rhs: TxId) -> Bool {
        lhs.isEqual(other: rhs)
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(asHashString())
    }
}
