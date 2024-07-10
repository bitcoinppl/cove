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
                    VerifyReminder(walletId: id, isVerified: model.isVerified)
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
                        .foregroundColor(.primary)
                        .padding()
                }
                // .frame(maxWidth: .infinity)
                .background(Color.yellow.opacity(0.6))
                .shadow(radius: 2)
                .enableInjection()
            }
        }
    }
}

#Preview {
    SelectedWalletView(id: WalletId())
}
