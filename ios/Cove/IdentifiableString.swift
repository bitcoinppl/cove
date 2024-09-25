//
//  IdentifiableString.swift
//  Cove
//
//  Created by Praveen Perera on 9/23/24.
//

import Foundation
import SwiftUI

struct IdentifiableString: Identifiable, Equatable {
    let id = UUID()
    let value: String

    init(_ value: String) {
        self.value = value
    }
}
