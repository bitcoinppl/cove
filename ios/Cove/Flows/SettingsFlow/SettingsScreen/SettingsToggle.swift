//
//  SettingsToggle.swift
//  Cove
//
//  Created by Praveen Perera on 2/3/25.
//

import SwiftUI

struct SettingsToggle: View {
    let title: String
    let symbol: String
    @Binding var item: Bool

    var body: some View {
        Toggle(isOn: $item) {
            HStack {
                SettingsIcon(symbol: symbol)
                Text(title)
                    .font(.subheadline)
                    .padding(8)
            }
            .padding(.vertical, 1)
        }
    }
}

#Preview {
    SettingsToggle(title: "Currency", symbol: "dollarsign.circle", item: .constant(true))
}
