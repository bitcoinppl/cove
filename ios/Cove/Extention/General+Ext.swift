//
//  General+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 10/20/24.
//

import CoveCore
import Foundation
import SwiftUI

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

    var isCustom: Bool {
        feeSpeedIsCustom(feeSpeed: self)
    }
}
