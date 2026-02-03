//
//  View+GlassEffect.swift
//  Cove
//
//  Created by Praveen Perera
//

import SwiftUI

extension View {
    /// Applies a glass effect when running on iOS 26+ and does nothing on older OSes.
    ///
    /// Note: Calling the system `glassEffect()` modifier directly at call sites requires
    /// availability checks (it's only available on iOS 26+). This convenience wrapper
    /// lets call sites use `.applyGlassEffect()` without sprinkling guards. On older OSes
    /// this is a no-op to preserve layout.
    @ViewBuilder
    func applyGlassEffect() -> some View {
        if #available(iOS 26.0, *) {
            self.glassEffect()
        } else {
            self
        }
    }

    /// Applies a glass effect with custom parameters when running on iOS 26+.
    @available(iOS 26.0, *)
    func applyGlassEffect(
        _ glass: Glass,
        in shape: some Shape = DefaultGlassEffectShape()
    ) -> some View {
        self.glassEffect(glass, in: shape)
    }
}
