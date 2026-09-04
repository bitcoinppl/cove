//
//  General+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 10/20/24.
//

import CoveCore
import Foundation
import SwiftUI

extension FeeSpeed {
    var string: String {
        self.description
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
