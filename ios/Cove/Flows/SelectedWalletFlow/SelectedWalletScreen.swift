//
//  SelectedWalletScreen.swift
//  Cove
//
//  Created by Praveen Perera on 11/28/24.
//

import ActivityIndicatorView
import SwiftUI

private enum SheetState: Equatable {
    case receive
    case settings
    case chooseAddressType([FoundAddress])
}

struct SelectedWalletScreen: View {
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

    @ViewBuilder
    var MainContent: some View {
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
                .contentShape(.contextMenuPreview, RoundedRectangle(cornerRadius: 8).inset(by: -8))
                .contextMenu {
                    Button("Settings") {
                        sheetState = .init(.settings)
                    }
                }
            }

            ToolbarItemGroup(placement: .navigationBarTrailing) {
                Button(action: {
                    app.sheetState = .init(.qr)
                }) {
                    Image(systemName: "qrcode")
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

    var body: some View {
        VStack {
            ScrollView {
                MainContent
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
        .onChange(of: model.walletMetadata.discoveryState) {
            _,
                newValue in setSheetState(newValue)
        }
        .onAppear { setSheetState(model.walletMetadata.discoveryState) }
        .onAppear(perform: model.validateMetadata)
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
                Button(action: {
                    navigate(Route.newWallet(.hotWallet(.verifyWords(walletId))))
                }
                ) {
                    HStack(spacing: 20) {
                        Image(systemName: "exclamationmark.triangle")
                            .foregroundStyle(.red.opacity(0.85))
                            .fontWeight(.semibold)

                        Text("backup wallet")
                            .fontWeight(.semibold)
                            .font(.caption)

                        Image(systemName: "exclamationmark.triangle")
                            .foregroundStyle(.red.opacity(0.85))
                            .fontWeight(.semibold)
                    }
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity)
                    .background(
                        LinearGradient(
                            colors: [.orange.opacity(0.67), .yellow.opacity(0.96)],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        )
                    )
                    .foregroundStyle(.black.opacity(0.66))
                }
            }
        }
    }
}

#Preview {
    AsyncPreview {
        NavigationStack {
            SelectedWalletScreen(model: WalletViewModel(preview: "preview_only"))
                .environment(MainViewModel())
        }
    }
}
