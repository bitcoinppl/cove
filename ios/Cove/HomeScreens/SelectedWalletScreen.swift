//
//  SelectedWalletScreen.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import ActivityIndicatorView
import SwiftUI

struct SelectedWalletScreen: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    let id: WalletId
    @State private var model: WalletViewModel? = nil

    func loadModel() {
        if model != nil { return }

        do {
            Log.debug("Getting wallet \(id)")
            model = try app.getWalletViewModel(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    var body: some View {
        Group {
            if let model = model {
                SelectedWalletScreenInner(model: model)
            } else {
                Text("Loading...")
            }
        }
        .onAppear {
            loadModel()
        }
        .task {
            // small delay and then start scanning wallet
            if let model = self.model {
                do {
                    try? await Task.sleep(for: .milliseconds(200))
                    try await model.rust.startWalletScan()
                } catch {
                    Log.error("Wallet Scan Failed \(error.localizedDescription)")
                }
            }
        }
        .onChange(of: model?.loadState) { _, loadState in
            if case .loaded = loadState {
                if let model = model {
                    app.updateWalletVm(model)
                }
            }
        }
        .tint(.white)
    }
}

struct SelectedWalletScreenInner: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    private let screenHeight = UIScreen.main.bounds.height

    // public
    var model: WalletViewModel

    // private
    @State private var showSettings = false
    @State private var receiveSheetShowing = false
    @State private var showingCopiedPopup = true

    func updater(_ action: WalletViewModelAction) {
        model.dispatch(action: action)
    }

    @ViewBuilder
    func transactionsCard(transactions: [Transaction], scanComplete: Bool) -> some View {
        TransactionsCardView(transactions: transactions, scanComplete: scanComplete, metadata: model.walletMetadata)
            .background(.thickMaterial)
            .ignoresSafeArea()
            .padding(.top, 10)
    }

    @ViewBuilder
    var Loading: some View {
        Spacer()
        ActivityIndicatorView(isVisible: Binding.constant(true), type: .default(count: 8))
            .frame(width: 30, height: 30)
            .padding(.top, screenHeight / 6)
        Spacer()
        Spacer()
    }

    func DisplayErrorAlert(_ alert: WalletErrorAlert) -> Alert {
        switch alert {
        case .nodeConnectionFailed:
            Alert(
                title: Text("Node Connection Failed"),
                message: Text("Would you like to select a different node?"),
                primaryButton: .default(Text("Yes, Change Node"), action: { navigate(.settings) }),
                secondaryButton: .cancel()
            )
        }
    }

    @ViewBuilder
    var Transactions: some View {
        switch model.loadState {
        case .loading:
            Loading
        case let .scanning(txns):
            if !model.walletMetadata.performedFullScan && txns.isEmpty {
                Loading
            } else {
                transactionsCard(transactions: txns, scanComplete: false)
            }
        case let .loaded(txns):
            transactionsCard(transactions: txns, scanComplete: true)
        }
    }

    var body: some View {
        VStack {
            VerifyReminder(walletId: model.walletMetadata.id, isVerified: model.walletMetadata.verified)

            ScrollView {
                VStack {
                    WalletBalanceHeaderView(balance: model.balance.confirmed,
                                            metadata: model.walletMetadata,
                                            updater: updater,
                                            receiveSheetShowing: $receiveSheetShowing)
                        .cornerRadius(16)
                        .padding()

                    Transactions
                        .environment(model)
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
                .sheet(isPresented: $receiveSheetShowing) {
                    ReceiveView(model: model)
                }
                .sheet(isPresented: $showSettings) {
                    WalletSettingsSheet(model: model)
                }
            }
            .refreshable {
                try? await model.rust.forceWalletScan()
                let _ = try? await model.rust.forceUpdateHeight()
            }
        }
        .alert(item: Binding(get: { model.errorAlert }, set: { model.errorAlert = $0 }), content: DisplayErrorAlert)
    }
}

struct VerifyReminder: View {
    @Environment(\.navigate) private var navigate
    let walletId: WalletId
    let isVerified: Bool

    var body: some View {
        Group {
            if !isVerified {
                Text("verify wallet")
                    .font(.caption)
                    .foregroundColor(.primary.opacity(0.6))
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity)
                    .background(Color.yellow.gradient.opacity(0.75))
                    .onTapGesture {
                        navigate(Route.newWallet(.hotWallet(.verifyWords(walletId))))
                    }
            }
        }
    }
}

#Preview("Loading") {
    SelectedWalletScreen(id: WalletId())
        .environment(MainViewModel())
}

#Preview("Loaded Wallet") {
    AsyncPreview {
        SelectedWalletScreenInner(model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}
