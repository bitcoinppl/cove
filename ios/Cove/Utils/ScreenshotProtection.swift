//
//  ScreenshotProtection.swift
//  Cove
//

import SwiftUI
import UIKit

/// Overlays an invisible secure UITextField over the view.
/// iOS prevents the contents of a live isSecureTextEntry field from appearing
/// in screenshots or screen recordings, blanking the covered region.
struct ScreenshotProtection: UIViewRepresentable {
    func makeUIView(context _: Context) -> UIView {
        let field = UITextField()
        field.isSecureTextEntry = true
        field.isUserInteractionEnabled = false
        field.alpha = 0
        return field
    }

    func updateUIView(_: UIView, context _: Context) {}
}

extension View {
    func screenshotProtected() -> some View {
        overlay(ScreenshotProtection().allowsHitTesting(false))
    }
}
