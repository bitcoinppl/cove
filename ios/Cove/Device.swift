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
            return Self.installedBuildChannel()
        #endif
    }

    /// Kept outside `#if DEBUG` so every configuration compiles it
    private static func installedBuildChannel() -> BuildChannel {
        // TestFlight installs carry a sandbox receipt, App Store installs don't
        if Bundle.main.appStoreReceiptURL?.lastPathComponent == "sandboxReceipt" {
            return .beta
        }

        return .release
    }
}
