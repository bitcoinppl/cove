import SwiftUI

struct WalletSettingsView: View {
    let model: WalletViewModel
    @Environment(\.navigate) private var navigate
    @Environment(\.presentationMode) var presentationMode

    @State private var showingDeleteConfirmation = false

    let colors: [WalletColor] = WalletColor.red.all()

    var body: some View {
        NavigationView {
            List {
                Section(header: Text("Wallet Information")) {
                    HStack {
                        Text("Network")
                        Spacer()
                        Text(model.walletMetadata.network.toString())
                            .foregroundColor(.secondary)
                    }
                    HStack {
                        Text("Fingerprint")
                        Spacer()
                        Text(model.rust.fingerprint())
                            .foregroundColor(.secondary)
                    }
                }

                Section(header: Text("Basic Settings")) {
                    TextField("Wallet Name", text: Binding(
                        get: { model.walletMetadata.name },
                        set: { model.dispatch(action: .updateName($0)) }
                    ))

                    Picker("Wallet Color", selection: Binding(
                        get: { model.walletMetadata.color },
                        set: { model.dispatch(action: .updateColor($0)) }
                    )) {
                        ForEach(colors, id: \.self) { color in
                            Text(color.toColor().description)
                                .tag(color)
                        }
                    }
                    .pickerStyle(MenuPickerStyle())
                    .tint(model.walletMetadata.color.toColor())
                }

                Section(header: Text("App Settings")) {
                    Button(action: {
                        presentationMode.wrappedValue.dismiss()
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
            .navigationBarItems(leading:
                Button {
                    presentationMode.wrappedValue.dismiss()
                    navigate(Route.settings)
                } label: {
                    Label("App Settings", systemImage: "gear")
                        .foregroundColor(.blue)
                }
            )
            .navigationBarItems(trailing: Button("Done") {
                presentationMode.wrappedValue.dismiss()
            })
            .foregroundColor(.primary)
            .confirmationDialog("Are you sure?", isPresented: $showingDeleteConfirmation) {
                Button("Delete", role: .destructive) {
                    do {
                        try model.rust.deleteWallet()
                        presentationMode.wrappedValue.dismiss()
                    } catch {
                        Log.error("Unable to delete wallet: \(error)")
                    }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This action cannot be undone.")
            }
        }
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}
