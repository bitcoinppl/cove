//
//  Constants.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import SwiftUI

let minSendSatsU = ffiMinSendSats()
let minSendAmount = ffiMinSendAmount()

let minSendSats = Int(minSendSatsU)

let screenHeight = UIScreen.main.bounds.height
let screenWidth = UIScreen.main.bounds.width

let isMiniDevice = screenHeight <= 812

func isMiniDeviceOrLargeText(_ sizeCategory: ContentSizeCategory) -> Bool {
    isMiniDevice || sizeCategory >= .extraExtraLarge
}
