import SwiftUI

struct WalletSettingsView: View {
    let manager: WalletManager
    @Environment(\.navigate) private var navigate
    @Environment(\.dismiss) private var dismiss

    @State private var showingDeleteConfirmation = false
    @State private var showingSecretWordsConfirmation = false

    let colors: [WalletColor] = WalletColor.red.all()

    var body: some View {
        List {
            Section(header: Text("Wallet Information")) {
                HStack {
                    Text("Network")
                    Spacer()
                    Text(manager.walletMetadata.network.toString())
                        .foregroundColor(.secondary)
                }
                .font(.subheadline)

                HStack {
                    Text("Fingerprint")
                    Spacer()
                    Text(manager.rust.fingerprint())
                        .foregroundColor(.secondary)
                }
                .font(.subheadline)
            }

            Section(header: Text("Settings")) {
                TextField(
                    "Wallet Name",
                    text: Binding(
                        get: { manager.walletMetadata.name },
                        set: { manager.dispatch(action: .updateName($0)) }
                    )
                )
                .font(.subheadline)

                Picker(
                    "Wallet Color",
                    selection: Binding(
                        get: { manager.walletMetadata.color },
                        set: { manager.dispatch(action: .updateColor($0)) }
                    )
                ) {
                    ForEach(colors, id: \.self) { color in
                        Text(color.toColor().description)
                            .tag(color)
                            .font(.subheadline)
                    }
                }
                .pickerStyle(MenuPickerStyle())
                .tint(manager.walletMetadata.color.toColor())
            }

            Section(header: Text("Danger Zone")) {
                if manager.walletMetadata.walletType == .hot {
                    Button {
                        showingSecretWordsConfirmation = true
                    } label: {
                        Text("View Secret Words")
                            .font(.subheadline)
                    }
                }

                Button {
                    showingDeleteConfirmation = true
                } label: {
                    Text("Delete Wallet").foregroundStyle(.red)
                        .font(.subheadline)
                }
            }
            .navigationTitle(manager.walletMetadata.name)
            .navigationBarTitleDisplayMode(.inline)
            .foregroundColor(.primary)
            .confirmationDialog("Are you sure?", isPresented: $showingDeleteConfirmation) {
                Button("Delete", role: .destructive) {
                    do {
                        try manager.rust.deleteWallet()
                        dismiss()
                    } catch {
                        Log.error("Unable to delete wallet: \(error)")
                    }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This action cannot be undone.")
            }
            .confirmationDialog("Are you sure?", isPresented: $showingSecretWordsConfirmation) {
                Button("Show Me") {
                    dismiss()
                    navigate(Route.secretWords(manager.walletMetadata.id))
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(
                    "Whoever has access to your secret words, has access to your bitcoin. Please keep these safe, don't show them to anyone."
                )
            }
        }
        .onDisappear {
            manager.validateMetadata()
        }
    }
}

#Preview {
    AsyncPreview {
        WalletSettingsView(manager: WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
            .environment(\.navigate) { _ in
                ()
            }
    }
}
