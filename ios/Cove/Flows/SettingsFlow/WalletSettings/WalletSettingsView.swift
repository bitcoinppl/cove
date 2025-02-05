import SwiftUI

struct WalletSettingsView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate
    @Environment(\.dismiss) private var dismiss

    let manager: WalletManager

    @State private var showingDeleteConfirmation = false
    @State private var showingSecretWordsConfirmation = false

    init(manager: WalletManager) {
        self.manager = manager
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    let colorColumns = Array(repeating: GridItem(.flexible(), spacing: 0), count: 5)

    var body: some View {
        List {
            Section(header: Text("Wallet Information")) {
                HStack {
                    Text("Network")
                    Spacer()
                    Text(metadata.network.toString())
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
                HStack {
                    Text("Name")
                    Spacer()

                    Text(metadata.name)
                        .font(.subheadline)
                        .foregroundColor(.secondary)

                    Image(systemName: "chevron.right")
                        .foregroundColor(Color(UIColor.tertiaryLabel))
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .contentShape(Rectangle())
                .font(.subheadline)
                .onTapGesture {
                    app.pushRoute(Route.settings(.wallet(id: metadata.id, route: .changeName)))
                }

                VStack(spacing: 14) {
                    HStack {
                        Text("Wallet Color")
                            .font(.subheadline)
                        Spacer()
                    }

                    HStack {
                        Rectangle()
                            .fill(metadata.swiftColor)
                            .cornerRadius(10)
                            .frame(width: 80, height: 80)

                        LazyVGrid(columns: colorColumns, spacing: 20) {
                            ForEach(defaultWalletColors(), id: \.self) { color in
                                ZStack {
                                    if color == metadata.color {
                                        Circle()
                                            .stroke(Color(color).opacity(0.7), lineWidth: 2)
                                            .frame(width: 32, height: 32)
                                    }

                                    Circle()
                                        .fill(Color(color))
                                        .frame(width: 28, height: 28)
                                        .contentShape(Rectangle())
                                }
                                .onTapGesture { manager.dispatch(action: .updateColor(color)) }
                            }
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
                .padding(.vertical, 8)
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
                    app.pushRoute(Route.secretWords(manager.walletMetadata.id))
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(
                    "Whoever has access to your secret words, has access to your bitcoin. Please keep these safe, don't show them to anyone."
                )
            }
        }
        .onDisappear { manager.validateMetadata() }
        .onAppear { manager.validateMetadata() }
        .scrollContentBackground(.hidden)
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
