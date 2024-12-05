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
        case let .import(numberOfWords, importType):
            HotWalletImportScreen(numberOfWords: numberOfWords, importType: importType)
        case let .verifyWords(walletId):
            VerifyWordsContainer(id: walletId)
        }
    }
}

#Preview("Select") {
    NewHotWalletContainer(route: .select)
        .environment(MainViewModel())
}

#Preview("Create") {
    NewHotWalletContainer(route: .create(.twelve))
        .environment(MainViewModel())
}

#Preview("Import") {
    NewHotWalletContainer(route: .import(.twelve, .manual))
        .environment(MainViewModel())
}
