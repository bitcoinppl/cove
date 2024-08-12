//
//  SelectedWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import ActivityIndicatorView
import SwiftUI

struct SelectedWalletView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    let id: WalletId
    @State private var model: WalletViewModel? = nil

    func loadModel() {
        if model != nil { return }

        do {
            Log.debug("Getting wallet \(id)")
            model = try WalletViewModel(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    var body: some View {
        Group {
            if let model = model {
                SelectedWalletViewInner(model: model)
            } else {
                Text("Loading...")
            }
        }
        .task {
            loadModel()

            if let model = self.model {
                do {
                    try await model.rust.startWalletScan()
                } catch {
                    Log.error("Wallet Scan Failed \(error.localizedDescription)")
                }
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

    // public
    let model: WalletViewModel

    // private
    @State private var showSettings = false

    func updater(_ action: WalletViewModelAction) {
        model.dispatch(action: action)
    }

    var accentColor: Color {
        Color(model.walletMetadata.color)
    }

    @ViewBuilder
    func transactionsCard(transactions: [Transaction], scanComplete: Bool) -> some View {
        TransactionsCardView(transactions: transactions, scanComplete: scanComplete, metadata: model.walletMetadata)
            .background(
                UnevenRoundedRectangle(
                    cornerRadii: .init(
                        topLeading: 40,
                        bottomLeading: 0,
                        bottomTrailing: 0,
                        topTrailing: 40
                    )
                )
                .fill(.thickMaterial)
                .ignoresSafeArea()
            )
    }

    @ViewBuilder
    var Transactions: some View {
        switch model.loadState {
        case .loading:
            Spacer()
            ActivityIndicatorView(isVisible: Binding.constant(true), type: .default(count: 8))
                .frame(width: 50, height: 50)
            Spacer()
            Spacer()
        case let .scanning(txns):
            transactionsCard(transactions: txns, scanComplete: false)
        case let .loaded(txns):
            transactionsCard(transactions: txns, scanComplete: true)
        }
    }

    var body: some View {
        ScrollView {
            VStack {
                WalletBalanceHeaderView(balance: model.balance.confirmed,
                                        metadata: model.walletMetadata,
                                        updater: updater)
                    .padding()

                Transactions
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
    AsyncPreview {
        SelectedWalletViewInner(model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}
