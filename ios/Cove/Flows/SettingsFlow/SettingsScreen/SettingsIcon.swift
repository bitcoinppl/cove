//
//  SettingsIcon.swift
//  Cove
//
//  Created by Praveen Perera on 1/30/25.
//

import SwiftUI

struct SettingsIcon: View {
    let symbol: String
    var foregroundColor: Color = .white
    var backgroundColor: Color = .gray
    var size: CGFloat = 22

    var body: some View {
        Image(systemName: symbol)
            .frame(width: size, height: size)
            .padding(5)
            .foregroundColor(foregroundColor)
            .background(backgroundColor)
            .cornerRadius(6)
    }
}

#Preview {
    VStack(spacing: 20) {
        SettingsIcon(symbol: "network", foregroundColor: .white, backgroundColor: .gray)
        SettingsIcon(symbol: "point.3.filled.connected.trianglepath.dotted")
    }
}
