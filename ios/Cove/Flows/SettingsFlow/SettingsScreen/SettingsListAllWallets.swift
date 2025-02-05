//
//  SettingsListAllWallets.swift
//  Cove
//
//  Created by Praveen Perera on 2/5/25.
//

import SwiftUI

struct SettingsListAllWallets: View {
    @State private var allWallets: [WalletMetadata] = []
    @State private var searchText = ""

    var filteredWallets: [WalletMetadata] {
        searchText.isEmpty ? allWallets : allWallets.filter {
            $0.name.localizedCaseInsensitiveContains(searchText)
        }
    }

    var body: some View {
        List(filteredWallets, id: \.self) { wallet in
            SettingsRow(
                title: wallet.name,
                route: .wallet(id: wallet.id, route: .main),
                icon: SettingsIcon(symbol: "wallet.bifold", backgroundColor: wallet.swiftColor)
            )
        }
        .navigationTitle("All Wallets")
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, prompt: "Search Wallets")
        .scrollContentBackground(.hidden)
        .onAppear {
            allWallets = (try? Database().wallets().allSortedActive()) ?? []
        }
    }
}

#Preview {
    SettingsListAllWallets()
}
