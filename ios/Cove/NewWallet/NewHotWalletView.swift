//
//  NewHotWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewHotWalletView: View {
    var route: HotWalletRoute

    var body: some View {
        switch route {
        case .select:
            HotWalletSelectView()
        case .create(words: let words):
            HotWalletCreateView(numberOfWords: words)
        case .import:
            HotWalletImportView()
        }
    }
}

#Preview("Select") {
    NewHotWalletView(route: HotWalletRoute.select)
}

#Preview("Create") {
    NewHotWalletView(route: HotWalletRoute.create(words: NumberOfBip39Words.twelve))
}

#Preview("Import") {
    NewHotWalletView(route: HotWalletRoute.import)
}
