//
//  ListWalletsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/24.
//

import SwiftUI

struct ListWalletsView: View {
    let model: MainViewModel
    @State var wallets: [WalletMetadata]
    @Environment(\.navigate) private var navigate

    init(model: MainViewModel) {
        self.model = model

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
                                (try? Fingerprint(id: wallet.id).toUppercase()) ?? "Unknown"
                            )
                            .foregroundColor(.white.opacity(0.7))
                            .font(.footnote)
                        }
                    }
                    .frame(width: 300, height: 200)
                    .onTapGesture {
                        try? model.rust.selectWallet(id: wallet.id)
                    }
                }
                .padding(.top, 10)
            }
        }
        .onAppear {
            if wallets.isEmpty {
                Log.debug("No wallets found, going to new wallet screen")
                model.resetRoute(to: RouteFactory().newWalletSelect())
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    ListWalletsView(model: MainViewModel())
}
