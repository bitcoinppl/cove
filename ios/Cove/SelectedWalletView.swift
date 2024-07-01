//
//  SelectedWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SelectedWalletView: View {
    var id: WalletId
    var walletMetadata: WalletMetadata?

    var body: some View {
        Group {
            if let walletMetadata = walletMetadata {
                Text(/*@START_MENU_TOKEN@*/"Hello, World!"/*@END_MENU_TOKEN@*/)
            } else {
                Text("Loading...")
            }
        }.onAppear {
            print("appeared")
        }
    }
}

#Preview {
    SelectedWalletView(id: WalletId())
}
