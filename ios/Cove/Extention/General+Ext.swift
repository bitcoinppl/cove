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

extension Address: Equatable {
    public static func == (lhs: Address, rhs: Address) -> Bool {
        addressIsEqual(lhs: lhs, rhs: rhs)
    }
}

extension FeeSpeed {
    var string: String {
        feeSpeedToString(feeSpeed: self)
    }

    var duration: String {
        feeSpeedDuration(feeSpeed: self)
    }

    var circleColor: Color {
        Color(feeSpeedToCircleColor(feeSpeed: self))
    }
}

extension FeeRateOption: Equatable {
    public static func == (lhs: FeeRateOption, rhs: FeeRateOption) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}

extension FeeRateOptionWithTotalFee: Equatable {
    public static func == (lhs: FeeRateOptionWithTotalFee, rhs: FeeRateOptionWithTotalFee) -> Bool {
        lhs.isEqual(rhs: rhs)
    }
}

extension FeeRateOptionsWithTotalFee: Equatable {
    public static func == (lhs: FeeRateOptionsWithTotalFee, rhs: FeeRateOptionsWithTotalFee) -> Bool {
        lhs.fast() == rhs.fast() && lhs.medium() == rhs.medium() && lhs.slow() == rhs.slow()
    }
}

extension Double {
    func btcFmt(maxDecimals: Int = 10) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = maxDecimals
        formatter.maximumFractionDigits = maxDecimals
        formatter.usesGroupingSeparator = false
        return formatter.string(from: NSNumber(value: self))!
    }

    func btcFmtWithUnit() -> String {
        btcFmt() + " BTC"
    }
}

extension Amount: Equatable {
    public static func == (lhs: Amount, rhs: Amount) -> Bool {
        lhs.asSats() == rhs.asSats()
    }
}

public extension SendRoute {
    func id() -> WalletId {
        switch self {
        case let .setAmount(id, address: _, amount: _): id
        case let .confirm(id: id, details: _): id
        }
    }
}

#if canImport(UIKit)
    extension View {
        func hideKeyboard() {
            UIApplication.shared.sendAction(
                #selector(UIResponder.resignFirstResponder),
                to: nil,
                from: nil,
                for: nil
            )
        }
    }
#endif
