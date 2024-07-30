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

    var body: some View {
        Group {
            if let model = model {
                SelectedWalletViewInner(model: model)
            } else {
                Text("Loading...")
            }
        }
        .onAppear {
            do {
                Log.debug("Getting wallet \(id)")
                model = try WalletViewModel(id: id)
            } catch {
                Log.error("Something went very wrong: \(error)")
                navigate(Route.listWallets)
            }
        }
        .tint(.white)
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

struct SelectedWalletViewInner: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    @State var model: WalletViewModel

    // private
    @State private var showSettings = false

    var body: some View {
        VStack {
            Spacer()

            Text("\(model.walletMetadata.name)")
                .foregroundColor(model.walletMetadata.color.toCardColors()[0].opacity(0.8))
                .font(.title2)

            Text(model.rust.fingerprint())

            Spacer()
            VerifyReminder(walletId: model.id, isVerified: model.isVerified)
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
        .task {
            do {
                try await model.rust.startWalletScan()
            } catch {
                Log.error("Wallet Scan Failed \(error.localizedDescription)")
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
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

#Preview("Loading") {
    SelectedWalletView(id: WalletId())
        .environment(MainViewModel())
}

#Preview("Loaded Wallet") {
    SelectedWalletViewInner(model: WalletViewModel(preview: "preview_only"))
        .environment(MainViewModel())
}
