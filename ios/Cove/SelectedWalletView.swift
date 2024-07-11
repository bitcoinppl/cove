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
    @State private var showingDeleteConfirmation = false
    @State private var showSettings = false

    func deleteWallet(model: WalletViewModel) {
        do {
            try model.rust.deleteWallet()
        } catch {
            Log.error("Unable to delete wallet: \(error)")
        }
    }

    var body: some View {
        Group {
            if let model = model {
                VStack {
                    Spacer()

                    Text("\(model.walletMetadata.name)")
                        .foregroundColor(model.walletMetadata.color.toCardColors()[0].opacity(0.8))
                        .font(.title2)

                    Text(model.rust.fingerprint())

                    Button(role: .destructive) {
                        showingDeleteConfirmation = true
                    } label: {
                        Image(systemName: "trash")
                        Text("Delete Wallet")
                            .bold()
                    }
                    .padding(.top, 20)
                    .confirmationDialog("Are you sure?", isPresented: $showingDeleteConfirmation) {
                        Button("Delete", role: .destructive) {
                            deleteWallet(model: model)
                        }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("This action cannot be undone.")
                    }

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
