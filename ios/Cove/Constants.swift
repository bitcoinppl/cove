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

let isMiniDevice = screenHeight <= 812
let compactLayoutHeightThreshold: CGFloat = 812

func isMiniDeviceOrLargeText(_ sizeCategory: ContentSizeCategory) -> Bool {
    isMiniDevice || sizeCategory >= .extraExtraLarge
}

func usesCompactLayout(sizeCategory: ContentSizeCategory, availableHeight: CGFloat) -> Bool {
    sizeCategory >= .extraExtraLarge || availableHeight <= compactLayoutHeightThreshold
}
