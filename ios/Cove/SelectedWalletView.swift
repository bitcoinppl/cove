//
//  SelectedWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SelectedWalletView: View {
    @Environment(\.navigate) private var navigate

    let id: WalletId
    @State private var model: WalletViewModel? = nil

    var body: some View {
        Group {
            if let model = model {
                VStack {
                    Text("NAME: \(model.walletMetadata.name)")
                        .foregroundColor(model.walletMetadata.color.toCardColors()[0])
                }
            } else {
                Text("Loading...")
            }
        }.onAppear {
            do {
                print("getting wallet for \(id)")
                model = try WalletViewModel(id: id)
            } catch {
                print("[SWIFT][ERROR] something went very wrong: \(error)")
                navigate(Route.listWallets)
            }
        }
    }
}

#Preview {
    SelectedWalletView(id: WalletId())
}
