//
//  FfiScanResultData.swift
//  Cove
//
//  Created by Praveen Perera on 9/23/24.
//

import Foundation

extension FfiScanResultData {
    init(_ scanResultData: ScanResultData) {
        switch scanResultData {
            case .string(let string):
                self = FfiScanResultData.string(string)
            case .data(let data):
                self = FfiScanResultData.data(data)
        }
    }
}
