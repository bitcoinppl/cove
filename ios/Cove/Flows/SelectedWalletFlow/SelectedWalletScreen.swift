//
//  SelectedWalletScreen.swift
//  Cove
//
//  Created by Praveen Perera on 11/28/24.
//

import SwiftUI
import SwiftUIIntrospect

struct ExportingBackup: Equatable {
    var tapSigner: TapSigner
    var backup: Data
}

enum SelectedWalletPresentationState: Equatable {
    case receive
    case chooseAddressType([FoundAddress])
    case qrLabelsImport
    case labelsFileImport
    case exportLabelsConfirmation
    case labelsQrExport
    case exportXpubConfirmation
    case xpubQrExport
}

struct SelectedWalletScreen: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate

    private let screenHeight = UIScreen.main.bounds.height

    /// nav bar height (~50) + scroll view system insets (~50)
    /// safeAreaInsets.top handles device-specific differences (notch, Dynamic Island)
    private let navBarAndScrollInsets: CGFloat = 100
    /// Delay long enough for SwiftUI to dismiss the title context menu before routing
    private let contextMenuDismissNavigationDelay: Duration = .milliseconds(350)

    /// public
    var manager: WalletManager

    @State private var presentationState: TaggedItem<SelectedWalletPresentationState>? = nil

    @State private var showingCopiedPopup = true
    @State private var shouldShowNavBar = false
    @State private var cloudBackupManager = CloudBackupManager.shared

    /// import / export
    @State var exportingBackup: ExportingBackup? = nil

    @State private var scannedLabels: TaggedItem<MultiFormat>? = nil
    @State private var pendingRenameNavigationTask: Task<Void, Never>? = nil

    /// private
    @State private var runPostRefresh = false

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var iOS26OrLater: Bool {
        if #available(iOS 26.0, *) { return true }
        return false
    }

    private var refreshControlTintColor: UIColor {
        UIColor.white
    }

    private func configureRefreshControl(in scrollView: UIScrollView) {
        configureRefreshControlIfAvailable(in: scrollView)

        DispatchQueue.main.async {
            self.configureRefreshControlIfAvailable(in: scrollView)
        }
    }

    private func configureRefreshControlIfAvailable(in scrollView: UIScrollView) {
        guard let refreshControl = scrollView.refreshControl else { return }

        refreshControl.tintColor = refreshControlTintColor
        refreshControl.backgroundColor = .clear

        // keep the indicator above the opaque hosted background while pulling at the top
        refreshControl.superview?.bringSubviewToFront(refreshControl)
    }

    func updater(_ action: WalletManagerAction) {
        manager.dispatch(action: action)
    }

    var labelManager: LabelManager {
        manager.rust.labelManager()
    }

    func transactionsCard(transactions: [CoveCore.Transaction]) -> some View {
        TransactionsCardView(
            transactions: transactions,
            unsignedTransactions: manager.unsignedTransactions,
            metadata: manager.walletMetadata
        )
        .ignoresSafeArea()
        .background(Color.coveBg)
    }

    @ViewBuilder
    var Transactions: some View {
        switch manager.loadState {
        case .loading:
            transactionsCard(transactions: [])
        case let .scanning(txns):
            transactionsCard(transactions: txns)
        case let .loaded(txns):
            transactionsCard(transactions: txns)
        }
    }

    private var refreshableTransactions: [CoveCore.Transaction]? {
        guard !manager.scanStatus.isActive else {
            return nil
        }

        return switch manager.loadState {
        case let .loaded(txns) where manager.ledgerState.initialScanComplete:
            txns
        case let .scanning(txns) where !manager.ledgerState.initialScanActive:
            txns
        case .loaded, .scanning, .loading:
            nil
        }
    }

    private var presentationContext: SelectedWalletPresentationContext {
        SelectedWalletPresentationContext(
            app: app,
            manager: manager,
            presentationState: $presentationState,
            walletErrorAlert: Binding(
                get: { manager.errorAlert },
                set: { manager.errorAlert = $0 }
            ),
            scannedLabels: $scannedLabels
        )
    }

    private var sheetPresentationState: Binding<TaggedItem<SelectedWalletPresentationState>?> {
        Binding(
            get: {
                guard let presentationState, presentationState.item.isSheet else { return nil }
                return presentationState
            },
            set: { newValue in
                if let newValue {
                    presentationState = newValue
                } else if presentationState?.item.isSheet == true {
                    presentationState = nil
                }
            }
        )
    }

    private var labelsFileImportIsPresented: Binding<Bool> {
        isPresenting(.labelsFileImport)
    }

    private var exportLabelsConfirmationIsPresented: Binding<Bool> {
        isPresenting(.exportLabelsConfirmation)
    }

    private var exportXpubConfirmationIsPresented: Binding<Bool> {
        isPresenting(.exportXpubConfirmation)
    }

    private func isPresenting(_ state: SelectedWalletPresentationState) -> Binding<Bool> {
        Binding(
            get: { presentationState?.item == state },
            set: { isPresented in
                if isPresented {
                    presentationState = TaggedItem(state)
                } else if presentationState?.item == state {
                    presentationState = nil
                }
            }
        )
    }

    private func setSheetState(_ discoveryState: DiscoveryState) {
        Log.debug("discoveryState: \(discoveryState)")

        switch discoveryState {
        case let .foundAddressesFromMnemonic(foundAddresses):
            presentationState = TaggedItem(.chooseAddressType(foundAddresses))
        case let .foundAddressesFromJson(foundAddress, _):
            presentationState = TaggedItem(.chooseAddressType(foundAddress))
        default: ()
        }
    }

    func showReceiveSheet() {
        presentationState = TaggedItem(.receive)
    }

    func showQrExport() {
        presentationState = TaggedItem(.labelsQrExport)
    }

    func presentXpubQrExport() {
        presentationState = TaggedItem(.xpubQrExport)
    }

    private func showRenameFromTitleMenu() {
        let walletId = metadata.id

        pendingRenameNavigationTask?.cancel()
        pendingRenameNavigationTask = Task { @MainActor in
            do {
                try await Task.sleep(for: contextMenuDismissNavigationDelay)
            } catch {
                return
            }

            app.pushRoute(Route.settings(.wallet(id: walletId, route: .changeName)))
        }
    }

    func shareXpubFile() {
        Task {
            do {
                let result = try await manager.rust.exportXpubForShare()
                ShareSheet.present(data: result.content, filename: result.filename) { success in
                    if !success {
                        Log.warn("Xpub Export Failed: cancelled or failed")
                    }
                }
            } catch {
                app.alertState = .init(
                    .general(
                        title: "Xpub Export Failed",
                        message:
                        "Unable to export public descriptors: \(error.localizedDescription)"
                    )
                )
            }
        }
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
                app.alertState = .init(
                    .general(
                        title: "Label Export Failed",
                        message: "Unable to export labels: \(error.localizedDescription)"
                    )
                )
            }
        }
    }

    private var toolbarTextColor: Color {
        if #available(iOS 26.0, *) {
            return shouldShowNavBar ? .primary : .white
        }
        return .white
    }

    @ToolbarContentBuilder
    var MainToolBar: some ToolbarContent {
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
                        importLabels: {
                            presentationState = TaggedItem(.labelsFileImport)
                        },
                        exportLabels: {
                            presentationState = TaggedItem(.exportLabelsConfirmation)
                        },
                        exportXpub: {
                            presentationState = TaggedItem(.exportXpubConfirmation)
                        }
                    )
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .adaptiveToolbarItemStyle(isPastHeader: shouldShowNavBar)
                        .font(.callout)
                }
                .accessibilityIdentifier("selectedWallet.more")
                .confirmationDialog(
                    "Export Labels",
                    isPresented: exportLabelsConfirmationIsPresented
                ) {
                    Button("QR Code") {
                        showQrExport()
                    }

                    Button("Share...") {
                        shareLabelsFile()
                    }

                    Button("Cancel", role: .cancel) {}
                }
                .confirmationDialog(
                    "Export Xpub",
                    isPresented: exportXpubConfirmationIsPresented
                ) {
                    Button("QR Code") {
                        presentXpubQrExport()
                    }

                    Button("Share...") {
                        shareXpubFile()
                    }

                    Button("Cancel", role: .cancel) {}
                }
            }
        }
    }

    private func handleHeaderBottomChanged(_ headerBottom: CGFloat) {
        let navBarThreshold = safeAreaInsets.top + 50
        let hysteresis: CGFloat = 10

        if !shouldShowNavBar, headerBottom < navBarThreshold - hysteresis {
            shouldShowNavBar = true
            app.isPastHeader = true
        } else if shouldShowNavBar, headerBottom > navBarThreshold + hysteresis {
            shouldShowNavBar = false
            app.isPastHeader = false
        }
    }

    private func selectedWalletMainContent() -> some View {
        SelectedWalletMainContent(
            manager: manager,
            screenHeight: screenHeight,
            cloudBackupIsConfigured: cloudBackupManager.isConfigured,
            updater: updater,
            showReceiveSheet: showReceiveSheet,
            headerBottomChanged: handleHeaderBottomChanged
        )
        .toolbar { MainToolBar }
        .navigationTitleView {
            SelectedWalletTitleContent(
                metadata: metadata,
                toolbarTextColor: toolbarTextColor,
                changeName: showRenameFromTitleMenu
            )
        }
        .adaptiveToolbarStyle(
            showNavBar: shouldShowNavBar,
            reduceTransparency: reduceTransparency
        )
        .presentingSheet(sheetPresentationState, context: presentationContext)
        .fileImporter(
            isPresented: labelsFileImportIsPresented,
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
        .onChange(of: scannedLabels, initial: false, onChangeOfScannedLabels)
    }

    func handleScrollToTransaction(proxy: ScrollViewProxy) {
        guard let targetId = manager.scrolledTransactionId else { return }
        if case .loading = manager.loadState { return }

        Task {
            await MainActor.run {
                withAnimation { proxy.scrollTo(targetId, anchor: .center) }
            }

            try? await Task.sleep(for: .milliseconds(500))
            if Task.isCancelled { return }
            await MainActor.run { manager.scrolledTransactionId = nil }
        }
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
            try manager.importLabels(labels: labels)
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
        ScrollViewReader { proxy in
            ScrollView {
                selectedWalletMainContent()
                    .background(
                        VStack(spacing: 0) {
                            Color.midnightBlue
                                .frame(height: screenHeight * 0.40 + 500)
                            Color.coveBg
                        }
                        .offset(y: -500)
                    )
            }
            .contentMargins(
                .top, -(safeAreaInsets.top + navBarAndScrollInsets), for: .scrollContent
            )
            .modifier(ScrollViewBackgroundModifier(iOS26OrLater: iOS26OrLater))
            .refreshable {
                // nothing to do – let the indicator disappear right away
                guard refreshableTransactions != nil else { return }
                let task = Task.detached { try? await Task.sleep(for: .seconds(1.75)) }

                // wait for the task to complete
                let _ = await task.result
                runPostRefresh = true // mark for later
            }
            .task(id: runPostRefresh) {
                guard runPostRefresh else { return }
                defer { runPostRefresh = false }
                guard let txns = refreshableTransactions else { return }

                self.manager.loadState = .scanning(txns)
                await manager.rust.forceWalletScan()
                let _ = try? await manager.rust.forceUpdateHeight()
                await manager.updateWalletBalance()
            }
            .introspect(.scrollView, on: .iOS(.v18, .v26)) { scrollView in
                configureRefreshControl(in: scrollView)
            }
            .onAppear {
                // Reset SendFlowManager so new send flow is fresh
                UIRefreshControl.appearance().tintColor = refreshControlTintColor
                app.clearSendFlowManager(id: manager.id)
                handleScrollToTransaction(proxy: proxy)
            }
            .onChange(of: manager.loadState, initial: true) {
                handleScrollToTransaction(proxy: proxy)
            }
            .scrollIndicators(.hidden)
            .modifier(SoftScrollEdgeModifier())
        }
        .modifier(OuterBackgroundModifier(iOS26OrLater: iOS26OrLater))
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
        .onAppear {
            shouldShowNavBar = false
            app.isPastHeader = false
        }
        .onDisappear {
            pendingRenameNavigationTask?.cancel()
            pendingRenameNavigationTask = nil

            app.isPastHeader = false
            UIRefreshControl.appearance().tintColor = UIColor.secondaryLabel
        }
        .presentingAlert(
            Binding(get: { manager.errorAlert }, set: { manager.errorAlert = $0 }),
            context: presentationContext,
            defaultTitle: "Error"
        )
        .environment(manager)
    }
}

extension SelectedWalletPresentationState {
    var isSheet: Bool {
        switch self {
        case .receive, .chooseAddressType, .qrLabelsImport, .labelsQrExport, .xpubQrExport:
            true
        case .labelsFileImport, .exportLabelsConfirmation, .exportXpubConfirmation:
            false
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
