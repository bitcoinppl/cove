//
//  SettingsRow.swift
//  Cove
//
//  Created by Praveen Perera on 2/4/25.
//

import SwiftUI

struct SettingsRow: View {
    @Environment(AppManager.self) private var app

    let title: Text
    var subtitle: Text? = nil

    var route: SettingsRoute? = nil

    var icon: SettingsIcon? = nil
    var symbol: String? = nil

    var onTapGesture: (() -> Void)? = nil

    init(
        title: LocalizedStringKey,
        subtitle: LocalizedStringKey? = nil,
        route: SettingsRoute? = nil,
        icon: SettingsIcon? = nil,
        symbol: String? = nil,
        onTapGesture: (() -> Void)? = nil
    ) {
        self.title = Text(title)
        self.subtitle = subtitle.map { Text($0) }
        self.route = route
        self.icon = icon
        self.symbol = symbol
        self.onTapGesture = onTapGesture
    }

    init(
        verbatimTitle title: String,
        route: SettingsRoute? = nil,
        icon: SettingsIcon? = nil,
        symbol: String? = nil,
        onTapGesture: (() -> Void)? = nil
    ) {
        self.title = Text(verbatim: title)
        self.route = route
        self.icon = icon
        self.symbol = symbol
        self.onTapGesture = onTapGesture
    }

    init(
        title: LocalizedStringKey,
        verbatimSubtitle subtitle: String,
        route: SettingsRoute? = nil,
        icon: SettingsIcon? = nil,
        symbol: String? = nil,
        onTapGesture: (() -> Void)? = nil
    ) {
        self.title = Text(title)
        self.subtitle = Text(verbatim: subtitle)
        self.route = route
        self.icon = icon
        self.symbol = symbol
        self.onTapGesture = onTapGesture
    }

    var body: some View {
        HStack {
            icon ?? SettingsIcon(symbol: symbol ?? "")

            VStack(alignment: .leading, spacing: 2) {
                title
                    .font(.subheadline)

                if let subtitle {
                    subtitle
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(8)

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
