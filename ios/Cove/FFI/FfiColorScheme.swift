//
//  FfiColorScheme.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import Foundation
import SwiftUI

extension FfiColorScheme {
    init(_ colorScheme: ColorScheme) {
        if colorScheme == .light {
            self = .light
        } else {
            self = .dark
        }
    }
}
