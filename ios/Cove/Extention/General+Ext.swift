//
//  General+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 10/20/24.
//

// General extention for types from rust
import Foundation
import SwiftUI

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

