import SwiftUI

struct WalletSettingsSheet: View {
    let manager: WalletManager
    @Environment(\.navigate) private var navigate
    @Environment(\.dismiss) private var dismiss

    // args
    @State var isSheet = true

    @State private var showingDeleteConfirmation = false
    @State private var showingSecretWordsConfirmation = false

    let colors: [WalletColor] = WalletColor.red.all()

    var body: some View {
        NavigationView {
            List {
                Section(header: Text("Wallet Information")) {
                    HStack {
                        Text("Network")
                        Spacer()
                        Text(manager.walletMetadata.network.toString())
                            .foregroundColor(.secondary)
                    }
                    HStack {
                        Text("Fingerprint")
                        Spacer()
                        Text(manager.rust.fingerprint())
                            .foregroundColor(.secondary)
                    }
                }

                Section(header: Text("Basic Settings")) {
                    TextField(
                        "Wallet Name",
                        text: Binding(
                            get: { manager.walletMetadata.name },
                            set: { manager.dispatch(action: .updateName($0)) }
                        )
                    )

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
                        }
                    }
                    .pickerStyle(MenuPickerStyle())
                    .tint(manager.walletMetadata.color.toColor())
                }

                Section(header: Text("App Settings")) {
                    Button(action: {
                        dismiss()
                        navigate(Route.settings)
                    }) {
                        HStack {
                            Text("App Settings")
                                .foregroundColor(.primary)

                            Spacer()

                            Image(systemName: "link")
                                .foregroundColor(.secondary)
                        }
                    }
                }

                Section(header: Text("Danger Zone")) {
                    if manager.walletMetadata.walletType == .hot {
                        Button {
                            showingSecretWordsConfirmation = true
                        } label: {
                            Label {
                                Text("View Secret Words")
                                    .foregroundColor(.orange)
                            } icon: {
                                Image(systemName: "lock.trianglebadge.exclamationmark")
                                    .foregroundColor(.orange)
                            }
                        }
                    }

                    Button {
                        showingDeleteConfirmation = true
                    } label: {
                        Label("Delete Wallet", systemImage: "trash")
                            .foregroundColor(.red)
                    }
                }
            }
            .listStyle(InsetGroupedListStyle())
            .navigationTitle("Wallet Settings")
            .toolbar {
                if isSheet {
                    ToolbarItem(placement: .topBarLeading) {
                        Button {
                            dismiss()
                            navigate(Route.settings)
                        } label: {
                            Label("App Settings", systemImage: "gear")
                                .foregroundColor(.blue)
                        }
                    }

                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Done") {
                            dismiss()
                            manager.validateMetadata()
                        }
                    }
                }
            }
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
        WalletSettingsSheet(manager: WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
            .environment(\.navigate) { _ in
                ()
            }
    }
}
