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
            wallets = try Database().wallets().getAll()
        } catch {
            print("[SWIFT][ERROR] Failed to get wallets \(error)")
            wallets = []
        }
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 20) {
                ForEach(wallets, id: \.id) { wallet in
                    GlassCard(colors: wallet.color.toCardColors()) {
                        Text(wallet.name).foregroundColor(.white)
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
                print("[SWIFT] Something went wrong, no wallets found")
                model.resetRoute(to: RouteFactory().newWalletSelect())
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.primary)
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    ListWalletsView(model: MainViewModel())
}
