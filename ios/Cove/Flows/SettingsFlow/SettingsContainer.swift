//
//  SettingsContainer.swift
//  Cove
//
//  Created by Praveen Perera on 1/29/25.
//

import SwiftUI

struct SettingsContainer: View {
    let route: SettingsRoute

    var body: some View {
        switch route {
        case .main:
            MainSettingsScreen()
        case .network:
            EmptyView()
        case .appearance:
            EmptyView()
        case .node:
            EmptyView()
        case .fiatCurrency:
            EmptyView()
        case .wallet(let walletId):
            WalletSettingsContainer(id: walletId)
        }
    }
}
