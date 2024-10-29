//
//  TaggedString.swift
//  Cove
//
//  Created by Praveen Perera on 9/23/24.
//

import Foundation
import SwiftUI

typealias TaggedString = TaggedItem<String>
extension TaggedString {
    var value: String {
        self.item
    }
}

struct TaggedItem<T: Equatable>: Identifiable, Equatable {
    let id = UUID()
    let item: T

    init(_ item: T) {
        self.item = item
    }
}

extension TaggedItem<StringOrData> {
    init(_ string: String) {
        let stringOrData = StringOrData(string)
        self.item = stringOrData
    }

    init(_ data: Data) {
        let stringOrData = StringOrData(data)
        self.item = stringOrData
    }
}
