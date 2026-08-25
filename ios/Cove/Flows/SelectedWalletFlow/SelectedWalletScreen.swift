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
        case let .foundAddressesFromXprv(foundAddresses):
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
                let result = try await manager.exportXpubForShare()
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
                let result = try await manager.exportLabelsForShare()
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
        SelectedWalletScrollReader(
            manager: manager,
            safeAreaTop: safeAreaInsets.top,
            navBarAndScrollInsets: navBarAndScrollInsets,
            iOS26OrLater: iOS26OrLater,
            runPostRefresh: $runPostRefresh,
            content: SelectedWalletPresentedContent(
                manager: manager,
                screenHeight: screenHeight,
                cloudBackupIsConfigured: cloudBackupManager.isConfigured,
                shouldShowNavBar: shouldShowNavBar,
                reduceTransparency: reduceTransparency,
                toolbarTextColor: toolbarTextColor,
                presentationState: $presentationState,
                sheetPresentationState: sheetPresentationState,
                labelsFileImportIsPresented: labelsFileImportIsPresented,
                exportLabelsConfirmationIsPresented: exportLabelsConfirmationIsPresented,
                exportXpubConfirmationIsPresented: exportXpubConfirmationIsPresented,
                scannedLabels: scannedLabels,
                presentationContext: presentationContext,
                updater: updater,
                showReceiveSheet: showReceiveSheet,
                headerBottomChanged: handleHeaderBottomChanged,
                changeName: showRenameFromTitleMenu,
                importLabelsFile: importLabelsFile,
                scannedLabelsChanged: onChangeOfScannedLabels,
                showLabelsQrExport: showQrExport,
                shareLabelsFile: shareLabelsFile,
                showXpubQrExport: presentXpubQrExport,
                shareXpubFile: shareXpubFile
            ),
            refresh: beginRefresh,
            performPostRefresh: performPostRefresh,
            configureRefreshControl: configureRefreshControl,
            prepareScroll: prepareScroll,
            scrollToTransaction: handleScrollToTransaction
        )
        .modifier(OuterBackgroundModifier(iOS26OrLater: iOS26OrLater))
        .onChange(of: manager.walletMetadata.discoveryState, discoveryStateChanged)
        .onAppear(perform: initializePresentation)
        .onAppear(perform: ensureWalletIsSelected)
        .onAppear(perform: resetHeaderState)
        .task {
            do {
                try await manager.validateMetadata()
            } catch is CancellationError {
                return
            } catch {
                Log.error("Unable to validate wallet metadata: \(error)")
            }
        }
        .onDisappear(perform: cleanUp)
        .presentingAlert(
            Binding(get: { manager.errorAlert }, set: { manager.errorAlert = $0 }),
            context: presentationContext,
            defaultTitle: "Error"
        )
        .environment(manager)
    }

    private func importLabelsFile(_ result: Result<URL, Error>) {
        do {
            let file = try result.get()
            let fileContents = try FileReader(for: file).read()
            try manager.labelManager().import(jsonl: fileContents)

            app.alertState = .init(
                .general(
                    title: "Success!",
                    message: "Labels have been imported successfully."
                )
            )

            Task { try? await manager.refreshTransactions() }
        } catch {
            app.alertState = .init(
                .general(
                    title: "Oops something went wrong!",
                    message: "Unable to import labels \(error.localizedDescription)"
                )
            )
        }
    }

    private func beginRefresh() async {
        guard refreshableTransactions != nil else { return }

        let task = Task.detached { try? await Task.sleep(for: .seconds(1.75)) }
        _ = await task.result
        runPostRefresh = true
    }

    private func performPostRefresh() async {
        guard runPostRefresh else { return }
        defer { runPostRefresh = false }
        guard let transactions = refreshableTransactions else { return }

        manager.loadState = .scanning(transactions)
        await manager.forceWalletScan()
        _ = try? await manager.forceUpdateHeight()
        await manager.updateWalletBalance()
    }

    private func prepareScroll(_ proxy: ScrollViewProxy) {
        UIRefreshControl.appearance().tintColor = refreshControlTintColor
        app.clearSendFlowManager(id: manager.id)
        handleScrollToTransaction(proxy: proxy)
    }

    private func initializePresentation() {
        setSheetState(manager.walletMetadata.discoveryState)
    }

    private func discoveryStateChanged(_: DiscoveryState, _ newValue: DiscoveryState) {
        setSheetState(newValue)
    }

    private func ensureWalletIsSelected() {
        guard Database().globalConfig().selectedWallet() != metadata.id else { return }

        Log.warn(
            "Wallet was not selected, but when to selected wallet screen, updating database"
        )
        try? Database().globalConfig().selectWallet(id: metadata.id)
    }

    private func resetHeaderState() {
        shouldShowNavBar = false
        app.isPastHeader = false
    }

    private func cleanUp() {
        pendingRenameNavigationTask?.cancel()
        pendingRenameNavigationTask = nil

        app.isPastHeader = false
        UIRefreshControl.appearance().tintColor = UIColor.secondaryLabel
    }
}

private struct SelectedWalletPresentedContent: View {
    let manager: WalletManager
    let screenHeight: CGFloat
    let cloudBackupIsConfigured: Bool
    let shouldShowNavBar: Bool
    let reduceTransparency: Bool
    let toolbarTextColor: Color
    @Binding var presentationState: TaggedItem<SelectedWalletPresentationState>?
    let sheetPresentationState: Binding<TaggedItem<SelectedWalletPresentationState>?>
    let labelsFileImportIsPresented: Binding<Bool>
    let exportLabelsConfirmationIsPresented: Binding<Bool>
    let exportXpubConfirmationIsPresented: Binding<Bool>
    let scannedLabels: TaggedItem<MultiFormat>?
    let presentationContext: SelectedWalletPresentationContext
    let updater: (WalletManagerAction) -> Void
    let showReceiveSheet: () -> Void
    let headerBottomChanged: (CGFloat) -> Void
    let changeName: () -> Void
    let importLabelsFile: (Result<URL, Error>) -> Void
    let scannedLabelsChanged: (
        TaggedItem<MultiFormat>?,
        TaggedItem<MultiFormat>?
    ) -> Void
    let showLabelsQrExport: () -> Void
    let shareLabelsFile: () -> Void
    let showXpubQrExport: () -> Void
    let shareXpubFile: () -> Void

    var body: some View {
        SelectedWalletMainContent(
            manager: manager,
            screenHeight: screenHeight,
            cloudBackupIsConfigured: cloudBackupIsConfigured,
            updater: updater,
            showReceiveSheet: showReceiveSheet,
            headerBottomChanged: headerBottomChanged
        )
        .toolbar {
            SelectedWalletToolbar(
                manager: manager,
                shouldShowNavBar: shouldShowNavBar,
                presentationState: $presentationState,
                exportLabelsConfirmationIsPresented: exportLabelsConfirmationIsPresented,
                exportXpubConfirmationIsPresented: exportXpubConfirmationIsPresented,
                showLabelsQrExport: showLabelsQrExport,
                shareLabelsFile: shareLabelsFile,
                showXpubQrExport: showXpubQrExport,
                shareXpubFile: shareXpubFile
            )
        }
        .navigationTitleView {
            SelectedWalletTitleContent(
                metadata: manager.walletMetadata,
                toolbarTextColor: toolbarTextColor,
                changeName: changeName
            )
        }
        .adaptiveToolbarStyle(
            showNavBar: shouldShowNavBar,
            reduceTransparency: reduceTransparency
        )
        .presentingSheet(sheetPresentationState, context: presentationContext)
        .fileImporter(
            isPresented: labelsFileImportIsPresented,
            allowedContentTypes: [.plainText, .json],
            onCompletion: importLabelsFile
        )
        .onChange(of: scannedLabels, initial: false, scannedLabelsChanged)
    }
}

private struct SelectedWalletToolbar: ToolbarContent {
    @Environment(AppManager.self) private var app

    let manager: WalletManager
    let shouldShowNavBar: Bool
    @Binding var presentationState: TaggedItem<SelectedWalletPresentationState>?
    let exportLabelsConfirmationIsPresented: Binding<Bool>
    let exportXpubConfirmationIsPresented: Binding<Bool>
    let showLabelsQrExport: () -> Void
    let shareLabelsFile: () -> Void
    let showXpubQrExport: () -> Void
    let shareXpubFile: () -> Void

    var body: some ToolbarContent {
        ToolbarItemGroup(placement: .navigationBarTrailing) {
            HStack(spacing: 5) {
                Button(action: showQrCode) {
                    Image(systemName: "qrcode")
                        .adaptiveToolbarItemStyle(isPastHeader: shouldShowNavBar)
                        .font(.callout)
                }

                Menu {
                    MoreInfoPopover(
                        manager: manager,
                        importLabels: showLabelsFileImport,
                        exportLabels: showLabelsExportConfirmation,
                        exportXpub: showXpubExportConfirmation
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
                    Button("QR Code", action: showLabelsQrExport)
                    Button("Share...", action: shareLabelsFile)
                    Button("Cancel", role: .cancel) {}
                }
                .confirmationDialog(
                    "Export Xpub",
                    isPresented: exportXpubConfirmationIsPresented
                ) {
                    Button("QR Code", action: showXpubQrExport)
                    Button("Share...", action: shareXpubFile)
                    Button("Cancel", role: .cancel) {}
                }
            }
        }
    }

    private func showQrCode() {
        app.sheetState = .init(.qr)
    }

    private func showLabelsFileImport() {
        presentationState = TaggedItem(.labelsFileImport)
    }

    private func showLabelsExportConfirmation() {
        presentationState = TaggedItem(.exportLabelsConfirmation)
    }

    private func showXpubExportConfirmation() {
        presentationState = TaggedItem(.exportXpubConfirmation)
    }
}

private struct SelectedWalletScrollReader<Content: View>: View {
    let manager: WalletManager
    let safeAreaTop: CGFloat
    let navBarAndScrollInsets: CGFloat
    let iOS26OrLater: Bool
    @Binding var runPostRefresh: Bool
    let content: Content
    let refresh: () async -> Void
    let performPostRefresh: () async -> Void
    let configureRefreshControl: (UIScrollView) -> Void
    let prepareScroll: (ScrollViewProxy) -> Void
    let scrollToTransaction: (ScrollViewProxy) -> Void

    var body: some View {
        ScrollViewReader { proxy in
            SelectedWalletScrollContent(
                manager: manager,
                safeAreaTop: safeAreaTop,
                navBarAndScrollInsets: navBarAndScrollInsets,
                iOS26OrLater: iOS26OrLater,
                runPostRefresh: runPostRefresh,
                content: content,
                refresh: refresh,
                performPostRefresh: performPostRefresh,
                configureRefreshControl: configureRefreshControl,
                prepareScroll: { prepareScroll(proxy) },
                scrollToTransaction: { scrollToTransaction(proxy) }
            )
        }
    }
}

private struct SelectedWalletScrollContent<Content: View>: View {
    let manager: WalletManager
    let safeAreaTop: CGFloat
    let navBarAndScrollInsets: CGFloat
    let iOS26OrLater: Bool
    let runPostRefresh: Bool
    let content: Content
    let refresh: () async -> Void
    let performPostRefresh: () async -> Void
    let configureRefreshControl: (UIScrollView) -> Void
    let prepareScroll: () -> Void
    let scrollToTransaction: () -> Void

    var body: some View {
        ScrollView {
            content
                .background(
                    SelectedWalletScrollBackground(
                        screenHeight: UIScreen.main.bounds.height
                    )
                )
        }
        .contentMargins(
            .top, -(safeAreaTop + navBarAndScrollInsets), for: .scrollContent
        )
        .modifier(ScrollViewBackgroundModifier(iOS26OrLater: iOS26OrLater))
        .refreshable {
            await refresh()
        }
        .task(id: runPostRefresh) {
            await performPostRefresh()
        }
        .introspect(
            .scrollView,
            on: .iOS(.v18, .v26),
            customize: configureRefreshControl
        )
        .onAppear(perform: prepareScroll)
        .onChange(of: manager.loadState, initial: true) {
            scrollToTransaction()
        }
        .scrollIndicators(.hidden)
        .modifier(SoftScrollEdgeModifier())
    }
}

private struct SelectedWalletScrollBackground: View {
    let screenHeight: CGFloat

    var body: some View {
        VStack(spacing: 0) {
            Color.midnightBlue
                .frame(height: screenHeight * 0.40 + 500)
            Color.coveBg
        }
        .offset(y: -500)
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

#Preview {
    AsyncPreview {
        NavigationStack {
            SelectedWalletScreen(
                manager: WalletManager(preview: .only)
            ).environment(AppManager.shared)
        }
    }
}
