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

    var route: SettingsRoute? = nil

    var icon: SettingsIcon? = nil
    var symbol: String? = nil

    var onTapGesture: (() -> Void)? = nil

    var body: some View {
        HStack {
            icon ?? SettingsIcon(symbol: symbol ?? "")

            Text(title)
                .font(.subheadline)
                .padding(8)

            if route != nil {
                Spacer()

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
