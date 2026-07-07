//
//  SettingsListAllWallets.swift
//  Cove
//
//  Created by Praveen Perera on 2/5/25.
//

import SwiftUI

struct SettingsListAllWallets: View {
    @Environment(AppManager.self) private var app

    @State private var allWallets: [WalletMetadata] = []
    @State private var searchText = ""

    var filteredWallets: [WalletMetadata] {
        searchText.isEmpty ? allWallets : allWallets.filter {
            $0.name.localizedCaseInsensitiveContains(searchText)
        }
    }

    private var isFiltering: Bool {
        !searchText.isEmpty
    }

    var body: some View {
        List {
            ForEach(filteredWallets, id: \.id) { wallet in
                SettingsRow(
                    title: wallet.name,
                    route: .wallet(id: wallet.id, route: .main),
                    icon: SettingsIcon(symbol: "wallet.bifold", backgroundColor: wallet.swiftColor)
                )
            }
            .onMove(perform: isFiltering ? nil : moveWallets)
            .moveDisabled(isFiltering)
        }
        .navigationTitle("All Wallets")
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, prompt: "Search Wallets")
        .scrollContentBackground(.hidden)
        .onAppear {
            syncWalletsFromApp()
        }
        .onChange(of: app.wallets) { _, wallets in
            allWallets = wallets
        }
        .onChange(of: searchText) { _, searchText in
            guard !searchText.isEmpty else { return }

            syncWalletsFromApp()
        }
    }

    private func moveWallets(from source: IndexSet, to destination: Int) {
        guard !isFiltering else { return }

        allWallets.move(fromOffsets: source, toOffset: destination)
        app.moveWallets(from: source, to: destination)
    }

    private func syncWalletsFromApp() {
        allWallets = app.wallets
    }
}

#Preview {
    SettingsListAllWallets()
        .environment(AppManager.shared)
}
