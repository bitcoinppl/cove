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
    case chooseAddressType([FoundAddress])
    case qrLabelsExport
    case qrLabelsImport
}

private enum AlertState: Equatable {
    case setupSuccess
    case exportSuccess
    case unableToImportLabels(String)
    case unableToExportLabels(String)
}

struct SelectedWalletScreen: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    @Environment(\.colorScheme) private var colorScheme
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate

    private let screenHeight = UIScreen.main.bounds.height

    // public
    var manager: WalletManager

    // private

    // alerts & sheets
    @State private var sheetState: TaggedItem<SheetState>? = nil
    @State private var alertState: TaggedItem<AlertState>? = nil

    @State private var showingCopiedPopup = true
    @State private var shouldShowNavBar = false

    // import / export
    @State private var isExportingLabels = false
    @State private var isImportingLabels = false
    @State private var scannedLabels: TaggedString? = nil

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    func updater(_ action: WalletManagerAction) {
        manager.dispatch(action: action)
    }

    var labelManager: LabelManager {
        manager.rust.labelManager()
    }

    @ViewBuilder
    func transactionsCard(transactions: [Transaction], scanComplete: Bool) -> some View {
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

    @ToolbarContentBuilder
    var MainToolBar: some ToolbarContent {
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
                        .foregroundStyle(.white)
                        .font(.callout)
                }

                Menu {
                    MoreInfoPopover(
                        manager: manager,
                        isExportingLabels: $isExportingLabels,
                        isImportingLabels: $isImportingLabels
                    )
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .foregroundStyle(.white)
                        .font(.callout)
                }
            }
        }
    }

    @ViewBuilder
    var MainContent: some View {
        VStack(spacing: 0) {
            WalletBalanceHeaderView(
                balance: manager.balance.total(),
                metadata: manager.walletMetadata,
                updater: updater,
                showReceiveSheet: showReceiveSheet
            )
            .clipped()
            .ignoresSafeArea(.all)

            VerifyReminder(
                walletId: manager.walletMetadata.id, isVerified: manager.walletMetadata.verified
            )

            Transactions
                .environment(manager)
        }
        .background(Color.coveBg)
        .toolbar { MainToolBar }
        .toolbarColorScheme(.dark, for: .navigationBar)
        .toolbarBackground(Color.midnightBlue, for: .navigationBar)
        .toolbarBackground(shouldShowNavBar ? .visible : .hidden, for: .navigationBar)
        .sheet(item: $sheetState, content: SheetContent)
        .fileExporter(
            isPresented: $isExportingLabels,
            document: JSONLDocument(text: exportLabelContent()),
            defaultFilename:
            labelManager.exportDefaultFileName(name: metadata.name)
        ) { result in
            switch result {
            case .success:
                alertState = .init(.exportSuccess)
            case let .failure(error):
                alertState = .init(.unableToExportLabels(error.localizedDescription))
            }
        }
        .fileImporter(
            isPresented: $isImportingLabels,
            allowedContentTypes: [.plainText, .json]
        ) { result in
            do {
                let file = try result.get()
                let fileContents = try FileReader(for: file).read()
                try labelManager.import(jsonl: fileContents)
                alertState = .init(.setupSuccess)

                // when labels are imported, we need to get the transactions again with the updated labels
                Task { await manager.rust.getTransactions() }
            } catch {
                alertState = .init(.unableToImportLabels(error.localizedDescription))
            }
        }
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: alertState,
            actions: { MyAlert($0).actions },
            message: { MyAlert($0).message }
        )
        .onChange(of: scannedLabels, initial: false, onChangeOfScannedLabels)
    }

    func onChangeOfScannedLabels(_: TaggedString?, _ labels: TaggedString?) {
        guard let labels else { return }
        do {
            try labelManager.import(jsonl: labels.item)
            alertState = .init(.setupSuccess)
        } catch {
            alertState = .init(.unableToImportLabels("Invalid QR code \(error.localizedDescription)"))
        }
    }

    func exportLabelContent() -> String {
        do {
            return try labelManager.export()
        } catch {
            alertState = .init(.unableToExportLabels(error.localizedDescription))
            return ""
        }
    }

    var body: some View {
        VStack {
            // set background colors below the scrollview
            ScrollView {
                MainContent
                    .background(
                        VStack {
                            Color.midnightBlue.frame(height: screenHeight * 0.40)
                            Color.coveBg
                        }
                    )
            }
            .refreshable {
                await manager.rust.forceWalletScan()
                let _ = try? await manager.rust.forceUpdateHeight()
                await manager.updateWalletBalance()
            }
            .onAppear { UIRefreshControl.appearance().tintColor = UIColor.white }
            .scrollIndicators(.hidden)
            .onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentOffset.y > (geometry.contentInsets.top + safeAreaInsets.top - 5)
            } action: { _, pastTop in
                shouldShowNavBar = pastTop
            }
        }
        .ignoresSafeArea(edges: .top)
        .onChange(of: manager.walletMetadata.discoveryState) { _, newValue in
            setSheetState(newValue)
        }
        .onAppear { setSheetState(manager.walletMetadata.discoveryState) }
        .onAppear {
            // make sure the wallet is marked as selected
            if Database().globalConfig().selectedWallet() != metadata.id {
                Log.warn("Wallet was not selected, but when to selected wallet screen, updating database")
                try? Database().globalConfig().selectWallet(id: metadata.id)
            }
        }
        .onAppear(perform: manager.validateMetadata)
        .alert(
            item: Binding(get: { manager.errorAlert }, set: { manager.errorAlert = $0 }),
            content: DisplayErrorAlert
        )
        .environment(manager)
    }

    // MARK: Alerts

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { alertState != nil },
            set: { if !$0 { alertState = .none } }
        )
    }

    private var alertTitle: String {
        guard let alert = alertState else { return "Error!" }
        return MyAlert(alert).title
    }

    private func MyAlert(_ alert: TaggedItem<AlertState>) -> AnyAlertBuilder {
        switch alert.item {
        case let .unableToImportLabels(error), let .unableToExportLabels(error):
            AlertBuilder(
                title: "Oops something went wrong!",
                message: error,
                actions: okButton
            ).eraseToAny()
        case .setupSuccess:
            AlertBuilder(
                title: "Success!",
                message: "Labels have been imported successfully.",
                actions: okButton
            ).eraseToAny()
        case .exportSuccess:
            AlertBuilder(
                title: "Success!",
                message: "Labels have been saved successfully.",
                actions: okButton
            )
            .eraseToAny()
        }
    }

    @ViewBuilder
    private func okButton() -> some View {
        Button("OK", action: { alertState = .none })
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
