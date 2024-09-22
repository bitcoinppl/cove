//
//  NewWalletContainer.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewWalletContainer: View {
    var route: NewWalletRoute
    @Environment(MainViewModel.self) private var mainViewModel

    var body: some View {
        switch route {
        case .select:
            NewWalletSelectScreen()
        case let .hotWallet(route):
            NewHotWalletContainer(route: route)
        case .coldWallet(.qrCode):
            QrCodeImportScreen()
        case .coldWallet(.nfc):
            Text("NFC import coming soon..")
        case .coldWallet(.file):
            Text("File import coming soon..")
        }
    }
}

#Preview {
    NewWalletContainer(route: NewWalletRoute.select).environment(MainViewModel())
}
