//
//  WalletSettingsChangeNameView.swift
//  Cove
//
//  Created by Praveen Perera on 2/5/25.
//

import SwiftUI

struct WalletSettingsChangeNameView: View {
    @Binding var name: String
    @FocusState private var isFocused: Bool

    var body: some View {
        Form {
            TextField("Wallet Name", text: $name)
                .modifier(ClearButton(text: $name))
                .focused($isFocused)
        }
        .scrollContentBackground(.hidden)
        .onAppear { isFocused = true }
    }
}

#Preview {
    WalletSettingsChangeNameView(name: Binding.constant("My Wallet"))
}
