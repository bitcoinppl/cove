//
//  WalletSettingsChangeNameView.swift
//  Cove
//
//  Created by Praveen Perera on 2/5/25.
//

import SwiftUI

struct WalletSettingsChangeNameView: View {
    let manager: WalletManager

    /// The field owns its text because the saved name only catches up after Rust reconciles each
    /// edit, and reading it back mid-typing would drop keystrokes
    @State private var name: String
    @FocusState private var isFocused: Bool

    init(manager: WalletManager) {
        self.manager = manager
        _name = State(initialValue: manager.walletMetadata.name)
    }

    var body: some View {
        Form {
            TextField("Wallet Name", text: $name)
                .modifier(ClearButton(text: $name))
                .focused($isFocused)
                .accessibilityIdentifier("walletSettings.changeName.field")
        }
        .scrollContentBackground(.hidden)
        .onAppear { isFocused = true }
        .onChange(of: name) { _, newName in
            manager.dispatch(action: .updateName(newName))
        }
    }
}

#Preview {
    WalletSettingsChangeNameView(manager: WalletManager(preview: .only))
}
