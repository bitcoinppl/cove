//
//  ListWalletsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/24.
//

import SwiftUI

struct ListWalletsScreen: View {
    let manager: AppManager
    @State var wallets: [WalletMetadata]
    @Environment(\.navigate) private var navigate

    init(manager: AppManager) {
        self.manager = manager

        do {
            wallets = try Database().wallets().all()
            Log.debug("Wallets: \(wallets)")
        } catch {
            Log.error("Failed to get wallets \(error)")
            wallets = []
        }
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 20) {
                ForEach(wallets, id: \.id) { wallet in
                    GlassCard(colors: wallet.color.toCardColors()) {
                        VStack {
                            Text(wallet.name).foregroundColor(.white).font(.title2)
                            Text(
                                (try? Fingerprint(id: wallet.id).asUppercase()) ?? "Unknown"
                            )
                            .foregroundColor(.white.opacity(0.7))
                            .font(.footnote)
                        }
                    }
                    .frame(width: 300, height: 200)
                    .onTapGesture {
                        try? manager.rust.selectWallet(id: wallet.id)
                    }
                }
                .padding(.top, 10)
            }
        }
        .onAppear {
            if manager.numberOfWallets < 2 {
                // wallet empty make a new one
                if wallets.isEmpty {
                    Log.debug("No wallets found, going to new wallet screen")
                    manager.resetRoute(to: RouteFactory().newWalletSelect())
                    return
                }

                // only has one wallet, so go directly to it
                if let wallet = wallets.first {
                    manager.resetRoute(to: Route.selectedWallet(wallet.id))
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .navigationTitle("Wallets")
        .background(.background)
        .padding(.top, 10)
    }
}

#Preview {
    ListWalletsScreen(manager: AppManager())
}
