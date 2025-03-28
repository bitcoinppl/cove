//
//  NewWalletContainer.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewWalletContainer: View {
    var route: NewWalletRoute

    var body: some View {
        switch route {
        case .select:
            NewWalletSelectScreen()
        case let .hotWallet(route):
            NewHotWalletContainer(route: route)
        case .coldWallet(.qrCode):
            QrCodeImportScreen()
        }
    }
}

#Preview {
    NewWalletContainer(route: NewWalletRoute.select).environment(AppManager.shared)
}
