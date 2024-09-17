//
//  FrozenColorScheme.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import Foundation
import SwiftUI

enum FrozenColorScheme {
    case light
    case dark

    init(_ colorScheme: ColorScheme) {
        switch colorScheme {
        case .light:
            self = FrozenColorScheme.light
        case .dark:
            self = FrozenColorScheme.dark
        default:
            self = FrozenColorScheme.light
        }
    }
}
