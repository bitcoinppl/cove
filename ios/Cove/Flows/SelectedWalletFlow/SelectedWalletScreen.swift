//
//  SelectedWalletScreen.swift
//  Cove
//
//  Created by Praveen Perera on 11/28/24.
//

import SwiftUI

struct ExportingBackup: Equatable {
    var tapSigner: TapSigner
    var backup: Data
}

private enum SheetState: Equatable {
    case receive
    case chooseAddressType([FoundAddress])
    case qrLabelsExport
    case qrLabelsImport
}

struct SelectedWalletScreen: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    @Environment(\.colorScheme) private var colorScheme
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate

    private let screenHeight = UIScreen.main.bounds.height

    // nav bar height (~50) + scroll view system insets (~50)
    // safeAreaInsets.top handles device-specific differences (notch, Dynamic Island)
    private let navBarAndScrollInsets: CGFloat = 100

    // public
    var manager: WalletManager

    // alerts & sheets
    @State private var sheetState: TaggedItem<SheetState>? = nil

    @State private var showingCopiedPopup = true
    @State private var shouldShowNavBar = false

    // import / export
    @State var exportingBackup: ExportingBackup? = nil

    @State private var scannedLabels: TaggedItem<MultiFormat>? = nil
    @State private var isImportingLabels = false
    @State private var showExportLabelsConfirmation = false
    @State private var showLabelsQrExport = false

    // private
    @State private var runPostRefresh = false

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    func updater(_ action: WalletManagerAction) {
        manager.dispatch(action: action)
    }

    var labelManager: LabelManager {
        manager.rust.labelManager()
    }

    func transactionsCard(transactions: [CoveCore.Transaction], scanComplete: Bool) -> some View {
        TransactionsCardView(
            transactions: transactions,
            unsignedTransactions: manager.unsignedTransactions,
            scanComplete: scanComplete,
            metadata: manager.walletMetadata
        )
        .ignoresSafeArea()
        .background(Color.coveBg)
    }

    @ViewBuilder
    var Loading: some View {
        Spacer()
        ProgressView()
            .padding(.top, screenHeight / 6)
            .tint(.primary)
        Spacer()
        Spacer()
    }

    func DisplayErrorAlert(_ alert: WalletErrorAlert) -> Alert {
        switch alert {
        case .nodeConnectionFailed:
            Alert(
                title: Text("Node Connection Failed"),
                message: Text("Would you like to select a different node?"),
                primaryButton: .default(
                    Text("Yes, Change Node"),
                    action: {
                        app.pushRoutes(RouteFactory().nestedSettings(route: .node))
                    }
                ),
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
        switch manager.loadState {
        case .loading:
            Loading
        case let .scanning(txns):
            if manager.walletMetadata.internal.lastScanFinished == nil, txns.isEmpty {
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
            ReceiveView(manager: manager)
        case let .chooseAddressType(foundAddresses):
            ChooseWalletTypeView(manager: manager, foundAddresses: foundAddresses)
        case .qrLabelsExport:
            EmptyView()
        case .qrLabelsImport:
            QrCodeLabelImportView(scannedCode: $scannedLabels)
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

    func showQrExport() {
        showLabelsQrExport = true
    }

    func shareLabelsFile() {
        Task {
            do {
                let result = try await manager.rust.exportLabelsForShare()
                ShareSheet.present(data: result.content, filename: result.filename) { success in
                    if !success {
                        Log.warn("Label Export Failed: cancelled or failed")
                    }
                }
            } catch {
                app.alertState = .init(.general(
                    title: "Label Export Failed",
                    message: "Unable to export labels: \(error.localizedDescription)"
                ))
            }
        }
    }

    @ToolbarContentBuilder
    var MainToolBar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            HStack(spacing: 10) {
                if case .cold = metadata.walletType {
                    BitcoinShieldIcon(width: 13, color: .white)
                }

                Text(metadata.name)
                    .foregroundStyle(.white)
                    .font(.callout)
                    .fontWeight(.semibold)
            }
            .padding(.vertical, 20)
            .padding(.horizontal, 28)
            .contentShape(Rectangle())
            .contentShape(
                .contextMenuPreview,
                RoundedRectangle(cornerRadius: 8)
            )
            .contextMenu {
                Button("Change Name") {
                    app.pushRoute(Route.settings(.wallet(id: metadata.id, route: .changeName)))
                }
            }
        }

        ToolbarItemGroup(placement: .navigationBarTrailing) {
            HStack(spacing: 5) {
                Button(action: {
                    app.sheetState = .init(.qr)
                }) {
                    Image(systemName: "qrcode")
                        .adaptiveToolbarItemStyle(isPastHeader: shouldShowNavBar)
                        .font(.callout)
                }

                Menu {
                    MoreInfoPopover(
                        manager: manager,
                        isImportingLabels: $isImportingLabels,
                        showExportLabelsConfirmation: $showExportLabelsConfirmation
                    )
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .adaptiveToolbarItemStyle(isPastHeader: shouldShowNavBar)
                        .font(.callout)
                }
                .confirmationDialog(
                    "Export Labels",
                    isPresented: $showExportLabelsConfirmation
                ) {
                    Button("QR Code") {
                        showQrExport()
                    }

                    Button("Share...") {
                        shareLabelsFile()
                    }

                    Button("Cancel", role: .cancel) {}
                }
            }
        }
    }

    var MainContent: some View {
        VStack(spacing: 0) {
            WalletBalanceHeaderView(
                balance: manager.balance.spendable(),
                metadata: manager.walletMetadata,
                updater: updater,
                showReceiveSheet: showReceiveSheet
            )
            .clipped()

            VerifyReminder(
                walletId: manager.walletMetadata.id, isVerified: manager.walletMetadata.verified
            )

            Transactions
                .environment(manager)
        }
        .background(Color.coveBg)
        .toolbar { MainToolBar }
        .adaptiveToolbarStyle(showNavBar: shouldShowNavBar)
        .sheet(item: $sheetState, content: SheetContent)
        .fileImporter(
            isPresented: $isImportingLabels,
            allowedContentTypes: [.plainText, .json]
        ) { result in
            do {
                let file = try result.get()
                let fileContents = try FileReader(for: file).read()
                try labelManager.import(jsonl: fileContents)

                app.alertState = .init(
                    .general(
                        title: "Success!",
                        message: "Labels have been imported successfully."
                    )
                )

                // when labels are imported, we need to get the transactions again with the updated labels
                Task { await manager.rust.getTransactions() }
            } catch {
                app.alertState = .init(
                    .general(
                        title: "Oops something went wrong!",
                        message: "Unable to import labels \(error.localizedDescription)"
                    )
                )
            }
        }
        .sheet(isPresented: $showLabelsQrExport) {
            QrExportView(
                title: "Export Labels",
                subtitle: "Scan to import labels\ninto another wallet",
                generateBbqrStrings: { density in
                    try await manager.rust.exportLabelsForQr(density: density)
                },
                generateUrStrings: nil
            )
            .presentationDetents([.height(500), .height(600), .large])
            .padding()
            .padding(.top, 10)
        }
        .onChange(of: scannedLabels, initial: false, onChangeOfScannedLabels)
    }

    func onChangeOfScannedLabels(_: TaggedItem<MultiFormat>?, _ scanned: TaggedItem<MultiFormat>?) {
        guard let scanned else { return }

        guard case let .bip329Labels(labels) = scanned.item else {
            app.alertState = .init(
                .general(
                    title: "Invalid QR Code",
                    message: "The scanned QR code does not contain BIP329 labels."
                )
            )
            return
        }

        do {
            try labelManager.importLabels(labels: labels)
            app.alertState = .init(
                .general(
                    title: "Success!",
                    message: "Labels have been imported successfully."
                )
            )

        } catch {
            app.alertState = .init(
                .general(
                    title: "Oops something went wrong!",
                    message: "Unable to import labels: \(error.localizedDescription)"
                )
            )
        }
    }

    var body: some View {
        VStack {
            ScrollViewReader { proxy in
                ScrollView {
                    MainContent
                        .background(
                            VStack(spacing: 0) {
                                Color.midnightBlue.frame(height: screenHeight * 0.40 + 500)
                                Color.coveBg
                            }
                            .offset(y: -500)
                        )
                }
                .contentMargins(.top, -(safeAreaInsets.top + navBarAndScrollInsets), for: .scrollContent)
                .background(Color.coveBg.ignoresSafeArea(edges: .bottom))
                .background(Color.midnightBlue.ignoresSafeArea(edges: .top))
                .refreshable {
                    // nothing to do – let the indicator disappear right away
                    guard case .loaded = manager.loadState else { return }
                    let task = Task.detached { try? await Task.sleep(for: .seconds(1.75)) }

                    // wait for the task to complete
                    let _ = await task.result
                    runPostRefresh = true // mark for later
                }
                .task(id: runPostRefresh) {
                    // runs when the flag flips
                    guard case let .loaded(txns) = manager.loadState else { return }
                    guard runPostRefresh else { return }
                    runPostRefresh = false

                    self.manager.loadState = .scanning(txns)
                    await manager.rust.forceWalletScan()
                    let _ = try? await manager.rust.forceUpdateHeight()
                    await manager.updateWalletBalance()
                }
                .onAppear {
                    // Reset SendFlowManager so new send flow is fresh
                    app.sendFlowManager = nil
                    UIRefreshControl.appearance().tintColor = UIColor.white
                }
                .onChange(of: manager.loadState, initial: true) { _, newState in
                    guard let targetId = manager.scrolledTransactionId else { return }

                    let hasTransactions: Bool = switch newState {
                    case .loading: false
                    case let .scanning(txns): !txns.isEmpty
                    case let .loaded(txns): !txns.isEmpty
                    }

                    guard hasTransactions else { return }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                        withAnimation {
                            proxy.scrollTo(targetId, anchor: .center)
                        }
                        manager.scrolledTransactionId = nil
                    }
                }
                .scrollIndicators(.hidden)
                .onScrollGeometryChange(for: Bool.self) { geometry in
                    geometry.contentOffset.y > (geometry.contentInsets.top + safeAreaInsets.top - 5)
                } action: { _, pastTop in
                    shouldShowNavBar = pastTop
                    app.isPastHeader = pastTop
                }
            }
        }
        .background(Color.midnightBlue.ignoresSafeArea())
        .onChange(of: manager.walletMetadata.discoveryState) { _, newValue in
            setSheetState(newValue)
        }
        .onAppear { setSheetState(manager.walletMetadata.discoveryState) }
        .onAppear {
            // make sure the wallet is marked as selected
            if Database().globalConfig().selectedWallet() != metadata.id {
                Log.warn(
                    "Wallet was not selected, but when to selected wallet screen, updating database"
                )
                try? Database().globalConfig().selectWallet(id: metadata.id)
            }
        }
        .onAppear(perform: manager.validateMetadata)
        .onDisappear {
            // reset scroll state when leaving this screen
            app.isPastHeader = false
        }
        .alert(
            item: Binding(get: { manager.errorAlert }, set: { manager.errorAlert = $0 }),
            content: DisplayErrorAlert
        )
        .environment(manager)
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
                    HStack(spacing: 20) {
                        Image(systemName: "exclamationmark.triangle")
                            .foregroundStyle(.red.opacity(0.85))
                            .fontWeight(.semibold)

                        Text("backup your wallet")
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
            SelectedWalletScreen(
                manager: WalletManager(preview: "preview_only")
            ).environment(AppManager.shared)
        }
    }
}
