//
//  Transition+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 3/20/25.
//

import SwiftUI

extension AnyTransition {
    static var navigationTransitionNext: AnyTransition {
        let insertion = AnyTransition.move(edge: .trailing)
        let removal = AnyTransition.move(edge: .leading)
        return .asymmetric(insertion: insertion, removal: removal)
    }

    static var navigationTransitionPrevious: AnyTransition {
        let insertion = AnyTransition.move(edge: .leading)
        let removal = AnyTransition.move(edge: .trailing)
        return .asymmetric(insertion: insertion, removal: removal)
    }
}
