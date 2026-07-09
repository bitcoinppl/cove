//
//  Constants.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import SwiftUI

let lowSendWarningSatsU = ffiLowSendWarningSats()
let lowSendWarningAmount = ffiLowSendWarningAmount()
let lowSendWarningSats = Int(lowSendWarningSatsU)

let conservativeDustLimitSatsU = ffiConservativeDustLimitSats()
let conservativeDustLimitAmount = ffiConservativeDustLimitAmount()
let conservativeDustLimitSats = Int(conservativeDustLimitSatsU)

let screenHeight = UIScreen.main.bounds.height
let screenWidth = UIScreen.main.bounds.width

let compactLayoutHeightThreshold: CGFloat = 812
let isMiniDevice = screenHeight <= compactLayoutHeightThreshold

/// Uses the global device height and Dynamic Type when sizing dense text controls
func usesCompactTypography(sizeCategory: ContentSizeCategory) -> Bool {
    isMiniDevice || sizeCategory >= .extraExtraLarge
}

/// Uses the current container height and Dynamic Type when choosing scrollable layout
func usesCompactLayout(sizeCategory: ContentSizeCategory, availableHeight: CGFloat) -> Bool {
    sizeCategory >= .extraExtraLarge || availableHeight <= compactLayoutHeightThreshold
}
