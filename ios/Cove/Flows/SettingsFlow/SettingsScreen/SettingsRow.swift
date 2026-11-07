//
//  SettingsRow.swift
//  Cove
//
//  Created by Praveen Perera on 2/4/25.
//

import SwiftUI

struct SettingsRow: View {
    @Environment(AppManager.self) private var app

    let title: String
    var subtitle: String? = nil

    var route: SettingsRoute? = nil

    var icon: SettingsIcon? = nil
    var symbol: String? = nil

    var onTapGesture: (() -> Void)? = nil

    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        HStack {
            // at accessibility sizes the icon left too little room for a title such as
            // "Appearance", which iOS split with a hyphen, so the title takes the icon's room
            if !dynamicTypeSize.isAccessibilitySize {
                icon ?? SettingsIcon(symbol: symbol ?? "")
            }

            SettingsRowTitle(title: title, subtitle: subtitle)

            Spacer()

            if route != nil {
                Image(systemName: "chevron.right")
                    .foregroundColor(Color(UIColor.tertiaryLabel))
                    .font(.footnote)
                    .fontWeight(.semibold)
            }
        }
        .padding(.vertical, 1)
        .contentShape(Rectangle())
        .onTapGesture {
            if let onTapGesture { return onTapGesture() }
            if let route { return app.pushRoute(Route.settings(route)) }
        }
    }
}

private struct SettingsRowTitle: View {
    let title: String
    let subtitle: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.subheadline)
                .uiTestTruncationProbe()

            if let subtitle, !subtitle.isEmpty {
                Text(subtitle)
                    .font(.caption2)
                    .uiTestTruncationProbe()
                    .foregroundStyle(.secondary)
            }
        }
        .padding(8)
    }
}

#Preview {
    VStack {
        Form {
            SettingsRow(title: "Currency", icon: SettingsIcon(symbol: "dollarsign.circle"))
            SettingsRow(title: "Currency", route: .fiatCurrency, icon: SettingsIcon(symbol: "dollarsign.circle"))
            SettingsRow(title: "Node", route: .node, icon: SettingsIcon(symbol: "point.3.filled.connected.trianglepath.dotted"))
        }
    }
    .environment(AppManager.shared)
}
