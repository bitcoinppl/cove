//
//  Shape+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 2025-05-20.
//

import SwiftUI

/// A Shape that insets (or outsizes) its rect by independent edge values.
struct DirectionalInsetShape<S: Shape>: Shape {
    let base: S
    let insets: EdgeInsets

    func path(in rect: CGRect) -> Path {
        // UIEdgeInsets is directional; negative insets “grow” the rect
        let uiInsets = UIEdgeInsets(
            top: insets.top,
            left: insets.leading,
            bottom: insets.bottom,
            right: insets.trailing
        )
        let insetRect = rect.inset(by: uiInsets)
        return base.path(in: insetRect)
    }
}

extension Shape {
    /// Insets (positive) or outsets (negative) each edge separately.
    func inset(by insets: EdgeInsets) -> some Shape {
        DirectionalInsetShape(base: self, insets: insets)
    }
}
