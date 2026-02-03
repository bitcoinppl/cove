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
    /// - iOS 26+: uses Liquid Glass design, lets system handle all styling
    /// - iOS 18 and earlier: maintains midnight blue background with dark color scheme
    @ViewBuilder
    func adaptiveToolbarStyle() -> some View {
        if #available(iOS 26.0, *) {
            // iOS 26+: let Liquid Glass system handle everything automatically
            // no custom background to avoid scroll edge effect conflicts
            self
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
    /// - iOS 26+: uses Liquid Glass design when visible, hides when not
    /// - iOS 18 and earlier: maintains midnight blue background with dark color scheme
    @ViewBuilder
    func adaptiveToolbarStyle(showNavBar: Bool) -> some View {
        if #available(iOS 26.0, *) {
            // iOS 26+: let system handle liquid glass automatically
            // don't override with clear background
            self
        } else {
            // iOS 18 and earlier: keep existing midnight blue style
            self
                .toolbarColorScheme(.dark, for: .navigationBar)
                .toolbarBackground(Color.midnightBlue, for: .navigationBar)
                .toolbarBackground(showNavBar ? .visible : .hidden, for: .navigationBar)
        }
    }

    /// applies adaptive foreground styling for toolbar items with scroll-based transitions
    ///
    /// - Parameters:
    ///   - isPastHeader: whether scrolled past the header threshold
    ///
    /// - iOS 26+: white over header, primary over content (follows Liquid Glass design)
    /// - iOS 18 and earlier: always white for midnight blue background visibility
    @ViewBuilder
    func adaptiveToolbarItemStyle(isPastHeader: Bool) -> some View {
        if #available(iOS 26.0, *) {
            // iOS 26+: transition from white (over header) to primary (over content)
            self.foregroundStyle(isPastHeader ? Color.primary : Color.white)
        } else {
            // iOS 18 and earlier: always white for visibility on midnight blue
            self.foregroundStyle(.white)
        }
    }
}

// MARK: - iOS 26 Tint Compatibility Modifiers

/// applies route-based tint colors only on iOS < 26
struct ConditionalRouteTintModifier: ViewModifier {
    let route: Route?

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            // iOS 26+: no tint applied, use system defaults (keeps toggles green)
            content
        } else {
            // iOS < 26: apply route-based tint colors
            let tintColor: Color = switch route {
            case .settings, .transactionDetails, .coinControl:
                .blue
            default:
                .white
            }
            content.tint(tintColor)
        }
    }
}

/// applies blue tint and accent color only on iOS < 26
struct RouteViewTintModifier: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
        } else {
            content
                .tint(.blue)
                .accentColor(.blue)
        }
    }
}

/// applies blue tint only on iOS < 26
struct ConditionalTintModifier: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
        } else {
            content.tint(.blue)
        }
    }
}

// MARK: - iOS 26 Scroll Edge Effect

/// applies soft scroll edge effect style on iOS 26+ for liquid glass cloud appearance
struct SoftScrollEdgeModifier: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.scrollEdgeEffectStyle(.soft, for: .top)
        } else {
            content
        }
    }
}

// MARK: - iOS 26 Background Modifiers

/// applies scroll view background - coveBg without safe area on iOS 26, with safe area on earlier
struct ScrollViewBackgroundModifier: ViewModifier {
    let iOS26OrLater: Bool

    func body(content: Content) -> some View {
        if iOS26OrLater {
            content.background(Color.coveBg)
        } else {
            content
                .background(Color.coveBg.ignoresSafeArea(edges: .bottom))
                .background(Color.midnightBlue.ignoresSafeArea(edges: .top))
        }
    }
}

/// applies outer background - coveBg on iOS 26, midnightBlue with safe area on earlier
struct OuterBackgroundModifier: ViewModifier {
    let iOS26OrLater: Bool

    func body(content: Content) -> some View {
        if iOS26OrLater {
            content.background(Color.coveBg)
        } else {
            content.background(Color.midnightBlue.ignoresSafeArea(edges: [.top, .bottom]))
        }
    }
}

// MARK: - NavBar Color Modifier

/// applies adaptive foreground styling to navigation bar items based on route and scroll state
struct NavBarColorModifier: ViewModifier {
    let route: Route
    let isPastHeader: Bool

    func body(content: Content) -> some View {
        switch route {
        case .selectedWallet:
            // use scroll-based adaptive styling for selectedWallet route
            content.adaptiveToolbarItemStyle(isPastHeader: isPastHeader)
        case .newWallet(.hotWallet(.create)), .newWallet(.hotWallet(.verifyWords)):
            // always white for these routes
            content.foregroundStyle(.white)
        default:
            // always white for all other routes
            content.foregroundStyle(.white)
        }
    }
}
