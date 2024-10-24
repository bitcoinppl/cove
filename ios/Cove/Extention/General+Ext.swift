//
//  General+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 10/20/24.
//

// General extention for types from rust

extension WalletAddressType: Comparable {
    public static func < (lhs: WalletAddressType, rhs: WalletAddressType) -> Bool {
        walletAddressTypeLessThan(lhs: lhs, rhs: rhs)
    }
}

extension DiscoveryState: Equatable {
    public static func == (lhs: DiscoveryState, rhs: DiscoveryState) -> Bool {
        discoveryStateIsEqual(lhs: lhs, rhs: rhs)
    }
}

public extension Transaction {
    func sentAndReceived() -> SentAndReceived {
        switch self {
            case .confirmed(let transaction):
                return transaction.sentAndReceived()
            case .unconfirmed(let transaction):
                return transaction.sentAndReceived()
        }
    }
}
