//
//  ListWalletsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/24.
//

import SwiftUI

struct ListWalletsView: View {
    @State var wallets: [WalletMetadata]
    @Environment(\.navigate) private var navigate

    init() {
        do {
            wallets = try Database().wallets().getAll()
        } catch {
            print("[SWIFT] Failed to get wallets \(error)")
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
                }
                .padding(.top, 10)
            }
        }
        .onAppear {
            if wallets.isEmpty {
                print("[SWIFT][ERROR] Something went wrong")
                navigate(RouteFactory().newWalletSelect())
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
    ListWalletsView()
}
