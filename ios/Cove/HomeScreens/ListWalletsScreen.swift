//
//  ListWalletsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/24.
//

import SwiftUI

struct ListWalletsScreen: View {
    @Environment(AppManager.self) private var app
    @State var wallets: [WalletMetadata]
    @Environment(\.navigate) private var navigate

    init() {
        do {
            wallets = try Database().wallets().all()
            Log.debug("Wallets: \(wallets)")
        } catch {
            Log.error("Failed to get wallets \(error)")
            wallets = []
        }
    }

    var body: some View {
        FullPageLoadingView()
            .onAppear {
                do {
                    if let wallet = wallets.first {
                        return try app.rust.selectWallet(id: wallet.id)
                    }
                } catch {
                    app.loadAndReset(to: Route.newWallet(.select))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

#Preview {
    ListWalletsScreen()
        .environment(AppManager.shared)
}
