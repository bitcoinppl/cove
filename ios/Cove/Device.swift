//
//  Device.swift
//  Cove
//
//  Created by Praveen Perera on 12/1/24.
//

import Foundation

class DeviceAccesor: DeviceAccess {
    func timezone() -> String {
        TimeZone.current.identifier
    }

    func buildChannel() -> BuildChannel {
        #if DEBUG
            return .debug
        #else
            // TestFlight installs carry a sandbox receipt, App Store installs don't
            if Bundle.main.appStoreReceiptURL?.lastPathComponent == "sandboxReceipt" {
                return .beta
            }

            return .release
        #endif
    }
}
