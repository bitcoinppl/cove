//
//  SettingsLink.swift
//  Cove
//
//  Created by Praveen Perera on 1/30/25.
//

import SwiftUI

struct SettingsLink: View {
    @Environment(AppManager.self) private var app

    let title: String
    let route: SettingsRoute
    var icon: SettingsIcon? = nil
    var symbol: String? = nil

    var body: some View {
        HStack {
            icon ?? SettingsIcon(symbol: symbol ?? "")

            Text(title)
                .font(.subheadline)
                .padding(8)

            Spacer()

            Image(systemName: "chevron.right")
                .foregroundColor(.secondary)
                .font(.footnote)
                .fontWeight(.semibold)
        }
        .padding(.vertical, 1)
        .contentShape(Rectangle())
        .onTapGesture {
            app.pushRoute(Route.settings(route))
        }
    }
}

#Preview {
    VStack {
        Form {
            SettingsLink(title: "Currency", route: .fiatCurrency, icon: SettingsIcon(symbol: "dollarsign.circle"))
            SettingsLink(title: "Node", route: .node, icon: SettingsIcon(symbol: "point.3.filled.connected.trianglepath.dotted"))
        }
    }
    .environment(AppManager.shared)
}
