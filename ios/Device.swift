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
}
