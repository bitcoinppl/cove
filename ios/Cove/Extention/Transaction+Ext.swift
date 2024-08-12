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
}

extension TxId: Hashable, Equatable {
    public static func == (lhs: TxId, rhs: TxId) -> Bool {
        lhs.isEqual(other: rhs)
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(self.toHashString())
    }
}
