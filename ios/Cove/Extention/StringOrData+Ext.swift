//
//  FfiScanResultData.swift
//  Cove
//
//  Created by Praveen Perera on 9/23/24.
//

import Foundation

extension StringOrData {
    init(_ scanResultData: ScanResultData) {
        switch scanResultData {
        case let .string(string):
            self = .string(string)
        case let .data(data):
            self = .data(data)
        }
    }

    init(_ value: String) {
        self.self = .string(value)
    }

    init(_ value: Data) {
        self.self = .data(value)
    }
}
