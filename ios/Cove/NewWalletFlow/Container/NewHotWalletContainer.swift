//
//  NewHotWalletContainer.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewHotWalletContainer: View {
    var route: HotWalletRoute

    var body: some View {
        switch route {
        case .select:
            HotWalletSelectScreen()
        case let .create(numberOfWords):
            HotWalletCreateScreen(numberOfWords: numberOfWords)
        case let .import(numberOfWords, scanning):
            HotWalletImportScreen(numberOfWords: numberOfWords, isPresentingScanner: scanning)
        case let .verifyWords(walletId):
            VerifyWordsScreen(id: walletId)
        }
    }
}

#Preview("Select") {
    NewHotWalletContainer(route: .select)
}

#Preview("Create") {
    NewHotWalletContainer(route: .create(.twelve))
}

#Preview("Import") {
    NewHotWalletContainer(route: .import(.twelve, false))
}
