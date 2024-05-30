//
//  SimpleBitcoinWalletUITestsLaunchTests.swift
//  SimpleBitcoinWalletUITests
//
//  Copyright 2024 BitcoinPPL.org and contributors.
//  Use of this source code is governed by the MIT license that can be found in the ./LICENSE.txt file.
//

import XCTest

final class SimpleBitcoinWalletUITestsLaunchTests: XCTestCase {

    override class var runsForEachTargetApplicationUIConfiguration: Bool {
        true
    }

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testLaunch() throws {
        let app = XCUIApplication()
        app.launch()

        // Insert steps here to perform after app launch but before taking a screenshot,
        // such as logging into a test account or navigating somewhere in the app

        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "Launch Screen"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
