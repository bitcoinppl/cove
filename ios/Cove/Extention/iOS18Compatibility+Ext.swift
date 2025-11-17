//
//  iOS18Compatibility+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 11/17/25.
//

import SwiftUI

// MARK: - iOS 18 Compatibility Extensions
// These extensions provide backward compatibility for iOS 18 while preparing for iOS 26+ designs
// Can be removed when minimum deployment target is iOS 26+

extension View {
    /// applies adaptive toolbar styling with always-visible navigation bar
    ///
    /// - iOS 26+: uses Liquid Glass design with translucent material
    /// - iOS 18 and earlier: maintains midnight blue background with dark color scheme
    @ViewBuilder
    func adaptiveToolbarStyle() -> some View {
        if #available(iOS 26.0, *) {
            // iOS 26+: use Liquid Glass design (no explicit background)
            // the system automatically applies translucent glass material
            self
                .toolbarBackground(.visible, for: .navigationBar)
        } else {
            // iOS 18 and earlier: keep existing midnight blue style
            self
                .toolbarColorScheme(.dark, for: .navigationBar)
                .toolbarBackground(Color.midnightBlue, for: .navigationBar)
                .toolbarBackground(.visible, for: .navigationBar)
        }
    }

    /// applies adaptive toolbar styling with conditional navigation bar visibility
    ///
    /// - Parameters:
    ///   - showNavBar: whether to show or hide the navigation bar
    ///
    /// - iOS 26+: uses Liquid Glass design with translucent material
    /// - iOS 18 and earlier: maintains midnight blue background with dark color scheme
    @ViewBuilder
    func adaptiveToolbarStyle(showNavBar: Bool) -> some View {
        if #available(iOS 26.0, *) {
            // iOS 26+: use Liquid Glass design (no explicit background)
            // the system automatically applies translucent glass material
            self
                .toolbarBackground(showNavBar ? .visible : .hidden, for: .navigationBar)
        } else {
            // iOS 18 and earlier: keep existing midnight blue style
            self
                .toolbarColorScheme(.dark, for: .navigationBar)
                .toolbarBackground(Color.midnightBlue, for: .navigationBar)
                .toolbarBackground(showNavBar ? .visible : .hidden, for: .navigationBar)
        }
    }
}
