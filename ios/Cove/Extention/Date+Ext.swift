//
//  Date+Ext.swift
//  Cove
//

import Foundation

extension Date {
    /// Formats a unix timestamp in seconds as an abbreviated date with a short time
    static func formattedTimestamp(_ timestamp: UInt64) -> String {
        Date(timeIntervalSince1970: TimeInterval(timestamp))
            .formatted(date: .abbreviated, time: .shortened)
    }
}
