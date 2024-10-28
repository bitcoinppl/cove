//
//  IdentifiableString.swift
//  Cove
//
//  Created by Praveen Perera on 9/23/24.
//

import Foundation
import SwiftUI

typealias IdentifiableString = IdentifiableItem<String>

extension IdentifiableString {
    var value: String {
        self.item
    }
}

struct IdentifiableItem<T: Equatable>: Identifiable, Equatable {
    let id = UUID()
    let item: T

    init(_ item: T) {
        self.item = item
    }
}

extension IdentifiableItem<StringOrData> {
    init(_ string: String) {
        let stringOrData = StringOrData(string)
        self.item = stringOrData
    }

    init(_ data: Data) {
        let stringOrData = StringOrData(data)
        self.item = stringOrData
    }
}
