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
        case let .create(numberOfWords):
            HotWalletCreateView(numberOfWords: numberOfWords)
        case let .import(numberOfWords):
            HotWalletImportView(numberOfWords: numberOfWords)
        case let .verifyWords(walletId):
            VerifyWordsView(id: walletId)
        }
    }
}

#Preview("Select") {
    NewHotWalletView(route: .select)
}

#Preview("Create") {
    NewHotWalletView(route: .create(.twelve))
}

#Preview("Import") {
    NewHotWalletView(route: .import(.twelve))
}
