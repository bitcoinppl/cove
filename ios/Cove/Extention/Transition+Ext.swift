//
//  Transition+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 3/20/25.
//

import SwiftUI

extension AnyTransition {
    static var navigationTransition: AnyTransition {
        .asymmetric(
            insertion: .move(edge: .trailing).animation(.easeInOut(duration: 0.3)),
            removal: .move(edge: .leading).animation(.easeInOut(duration: 0.3))
        )
    }
}
