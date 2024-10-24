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
        case .confirmed(let confirmedTransaction):
            confirmedTransaction.id()
        case .unconfirmed(let unconfirmedTransaction):
            unconfirmedTransaction.id()
        }
    }

    func sentAndReceived() -> SentAndReceived {
        switch self {
        case .confirmed(let transaction):
            return transaction.sentAndReceived()
        case .unconfirmed(let transaction):
            return transaction.sentAndReceived()
        }
    }
}

extension TxId: Hashable, Equatable {
    public static func == (lhs: TxId, rhs: TxId) -> Bool {
        lhs.isEqual(other: rhs)
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(self.asHashString())
    }
}
