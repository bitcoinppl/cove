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
        case let .create(words: words):
            HotWalletCreateView(numberOfWords: words)
        case .import:
            HotWalletImportView()
        case let .verifyWords(walletId):
            VerifyWordsView(id: walletId)
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
