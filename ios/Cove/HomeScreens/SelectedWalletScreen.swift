//
//  SelectedWalletScreen.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import ActivityIndicatorView
import SwiftUI

struct SelectedWalletScreen: View {
    @Environment(\.colorScheme) private var colorScheme
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
            if let model {
                SelectedWalletScreenInner(model: model)
                    .background(
                        model.loadState == .loading ?
                            LinearGradient(colors: [
                                .black.opacity(colorScheme == .dark ? 0.9 : 0),
                                .black.opacity(colorScheme == .dark ? 0.9 : 0)
                            ], startPoint: .top, endPoint: .bottom) :
                            LinearGradient(stops: [
                                .init(color: .midnightBlue, location: 0.45),
                                .init(color: colorScheme == .dark ? .black.opacity(0.9) : .clear, location: 0.55)
                            ], startPoint: .top, endPoint: .bottom)
                    )
                    .background(Color.white)

            } else {
                Text("Loading...")
            }
        }
        .onAppear {
            loadModel()
        }
        .task {
            // small delay and then start scanning wallet
            if let model {
                do {
                    try? await Task.sleep(for: .milliseconds(400))
                    try await model.rust.startWalletScan()
                } catch {
                    Log.error("Wallet Scan Failed \(error.localizedDescription)")
                }
            }
        }
        .onChange(of: model?.loadState) { _, loadState in
            if case .loaded = loadState {
                if let model {
                    app.updateWalletVm(model)
                }
            }
        }
    }
}

private enum SheetState: Equatable {
    case receive
    case settings
    case chooseAddressType([FoundAddress])
}

struct SelectedWalletScreenInner: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    @Environment(\.colorScheme) private var colorScheme
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    private let screenHeight = UIScreen.main.bounds.height

    // public
    var model: WalletViewModel

    // private
    @State private var sheetState: TaggedItem<SheetState>? = nil
    @State private var showingCopiedPopup = true
    @State private var shouldShowNavBar = false

    var metadata: WalletMetadata {
        model.walletMetadata
    }

    func updater(_ action: WalletViewModelAction) {
        model.dispatch(action: action)
    }

    @ViewBuilder
    func transactionsCard(transactions: [Transaction], scanComplete: Bool) -> some View {
        TransactionsCardView(
            transactions: transactions,
            unsignedTransactions: model.unsignedTransactions,
            scanComplete: scanComplete,
            metadata: model.walletMetadata
        )
        .background(colorScheme == .dark ? .black.opacity(0.9) : .clear)
        .background(Color.white)
        .ignoresSafeArea()
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
        case .noBalance:
            .init(
                title: Text("No Balance"),
                message: Text("Can't send a transaction, when you have no funds."),
                primaryButton: .default(
                    Text("Receive Funds"),
                    action: { sheetState = .init(.receive) }
                ),
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
            if !model.walletMetadata.performedFullScan, txns.isEmpty {
                Loading
            } else {
                transactionsCard(transactions: txns, scanComplete: false)
            }
        case let .loaded(txns):
            transactionsCard(transactions: txns, scanComplete: true)
        }
    }

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .receive:
            ReceiveView(model: model)
        case .settings:
            WalletSettingsSheet(model: model)
        case let .chooseAddressType(foundAddresses):
            ChooseWalletTypeView(model: model, foundAddresses: foundAddresses)
        }
    }

    private func setSheetState(_ discoveryState: DiscoveryState) {
        Log.debug("discoveryState: \(discoveryState)")

        switch discoveryState {
        case let .foundAddressesFromMnemonic(foundAddresses):
            sheetState = TaggedItem(.chooseAddressType(foundAddresses))
        case let .foundAddressesFromJson(foundAddress, _):
            sheetState = TaggedItem(.chooseAddressType(foundAddress))
        default: ()
        }
    }

    func showReceiveSheet() {
        sheetState = TaggedItem(.receive)
    }

    var body: some View {
        VStack {
            ScrollView {
                VStack(spacing: 0) {
                    WalletBalanceHeaderView(
                        balance: model.balance.confirmed,
                        metadata: model.walletMetadata,
                        updater: updater,
                        showReceiveSheet: showReceiveSheet
                    )
                    .clipped()
                    .ignoresSafeArea(.all)

                    VerifyReminder(
                        walletId: model.walletMetadata.id, isVerified: model.walletMetadata.verified
                    )

                    Transactions
                        .environment(model)
                }
                .toolbar {
                    ToolbarItem(placement: .principal) {
                        HStack(spacing: 10) {
                            if metadata.walletType == .cold {
                                BitcoinShieldIcon(width: 13, color: .white)
                            }

                            Text(metadata.name)
                                .foregroundStyle(.white)
                                .font(.callout)
                                .fontWeight(.semibold)
                        }
                    }

                    ToolbarItem(placement: .navigationBarTrailing) {
                        Button(action: {
                            sheetState = TaggedItem(.settings)
                        }) {
                            Image(systemName: "gear")
                                .foregroundStyle(.white)
                                .font(.callout)
                        }
                    }
                }
                .toolbarColorScheme(.dark, for: .navigationBar)
                .toolbarBackground(Color.midnightBlue.opacity(0.9), for: .navigationBar)
                .toolbarBackground(shouldShowNavBar ? .visible : .hidden, for: .navigationBar)
                .sheet(item: $sheetState, content: SheetContent)
            }
            .refreshable {
                try? await model.rust.forceWalletScan()
                let _ = try? await model.rust.forceUpdateHeight()
            }
            .onAppear {
                UIRefreshControl.appearance().tintColor = UIColor.white
            }
            .scrollIndicators(.hidden)
            .onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentOffset.y > (geometry.contentInsets.top + safeAreaInsets.top - 5)
            } action: { _, pastTop in
                shouldShowNavBar = pastTop
            }
        }
        .ignoresSafeArea(edges: .top)
        .onChange(of: model.walletMetadata.discoveryState) { _,
            newValue in setSheetState(newValue)
        }
        .onAppear { setSheetState(model.walletMetadata.discoveryState) }
        .alert(
            item: Binding(get: { model.errorAlert }, set: { model.errorAlert = $0 }),
            content: DisplayErrorAlert
        )
        .environment(model)
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
        NavigationStack {
            SelectedWalletScreenInner(model: WalletViewModel(preview: "preview_only"))
                .environment(MainViewModel())
        }
    }
}
