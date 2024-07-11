//
//  SelectedWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SelectedWalletView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    let id: WalletId
    @State private var model: WalletViewModel? = nil
    @State private var showSettings = false

    var body: some View {
        Group {
            if let model = model {
                VStack {
                    Spacer()

                    Text("\(model.walletMetadata.name)")
                        .foregroundColor(model.walletMetadata.color.toCardColors()[0].opacity(0.8))
                        .font(.title2)

                    Text(model.rust.fingerprint())

                    Spacer()
                    VerifyReminder(walletId: id, isVerified: model.isVerified)
                }
                .toolbar {
                    ToolbarItem(placement: .navigationBarTrailing) {
                        Button(action: {
                            showSettings = true
                        }) {
                            Image(systemName: "gear")
                                .foregroundColor(.primary.opacity(0.8))
                        }
                    }
                }
                .navigationTitle(model.walletMetadata.name)
                .toolbarColorScheme(.dark, for: .navigationBar)
                .toolbarBackground(model.walletMetadata.color.toColor(), for: .navigationBar)
                .toolbarBackground(.visible, for: .navigationBar)
                .sheet(isPresented: $showSettings) {
                    WalletSettingsView(model: model)
                }
            } else {
                Text("Loading...")
            }
        }
        .onAppear {
            do {
                model = try WalletViewModel(id: id)
            } catch {
                Log.error("Something went very wrong: \(error)")
                navigate(Route.listWallets)
            }
        }
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

struct WalletSettingsView: View {
    let model: WalletViewModel
    @Environment(\.navigate) private var navigate
    @Environment(\.presentationMode) var presentationMode

    @State private var showingDeleteConfirmation = false

    let colors: [WalletColor] = WalletColor.red.all()

    var body: some View {
        NavigationView {
            List {
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
                                .foregroundColor(.clear)
                                .background(color.toColor())
                                .frame(width: 30, height: 30)
                                .clipShape(Circle())
                                .tag(color)
                        }
                    }
                    .pickerStyle(MenuPickerStyle())
                }

                Section {
                    Button(action: {
                        presentationMode.wrappedValue.dismiss()
                        navigate(Route.settings)
                    }) {
                        Label("App Settings", systemImage: "gear")
                            .foregroundColor(.blue)
                    }
                }

                Section {
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
            .navigationBarItems(trailing: Button("Done") {
                presentationMode.wrappedValue.dismiss()
            })
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
    }
}

struct VerifyReminder: View {
    @Environment(\.navigate) private var navigate
    let walletId: WalletId
    let isVerified: Bool

    var body: some View {
        Group {
            if !isVerified {
                Button(action: {
                    navigate(Route.newWallet(.hotWallet(.verifyWords(walletId))))
                }) {
                    Text("verify wallet")
                        .font(.caption)
                        .foregroundColor(.primary.opacity(0.8))
                        .padding(.top, 20)
                }
                .frame(maxWidth: .infinity)
                .background(Color.yellow.gradient)
            }
        }
    }
}

#Preview {
    SelectedWalletView(id: WalletId())
}
