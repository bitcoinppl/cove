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

    /// nav bar height (~50) + scroll view system insets (~50)
    /// safeAreaInsets.top handles device-specific differences (notch, Dynamic Island)
    private let navBarAndScrollInsets: CGFloat = 100
    /// Delay long enough for SwiftUI to dismiss the title context menu before routing
    private let contextMenuDismissNavigationDelay: Duration = .milliseconds(350)

    /// public
    var manager: WalletManager

    /// alerts & sheets
    @State private var sheetState: TaggedItem<SheetState>? = nil

    @State private var showingCopiedPopup = true
    @State private var shouldShowNavBar = false
    @State private var cloudBackupManager = CloudBackupManager.shared

    /// import / export
    @State var exportingBackup: ExportingBackup? = nil

    @State private var scannedLabels: TaggedItem<MultiFormat>? = nil
    @State private var isImportingLabels = false
    @State private var showExportLabelsConfirmation = false
    @State private var showLabelsQrExport = false
    @State private var showExportXpubConfirmation = false
    @State private var showXpubQrExport = false
    @State private var pendingRenameNavigationTask: Task<Void, Never>? = nil

    /// private
    @State private var runPostRefresh = false
    @State private var torQuickStatus = TorQuickStatus()
    @State private var showTorQuickStatus = false

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var iOS26OrLater: Bool {
        if #available(iOS 26.0, *) { return true }
        return false
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

    func presentXpubQrExport() {
        showXpubQrExport = true
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

    @MainActor
    private func pollTorQuickStatus() async {
        while !Task.isCancelled {
            await refreshTorQuickStatus()
            try? await Task.sleep(for: .seconds(3))
        }
    }

    @MainActor
    private func refreshTorQuickStatus() async {
        let config = Database().globalConfig()
        guard config.useTor() else {
            torQuickStatus = TorQuickStatus()
            return
        }

        let mode = TorMode.fromConfig(try? config.get(key: .torMode))
        var quick = TorQuickStatus(enabled: true)

        switch mode {
        case .builtIn:
            await refreshBuiltInQuickStatus(quick: &quick)
        case .orbot:
            await refreshProxyQuickStatus(
                host: "127.0.0.1",
                port: 9050,
                modeTitle: "Orbot",
                quick: &quick
            )
        case .external:
            let host = (try? config.get(key: .torExternalHost))?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            await refreshProxyQuickStatus(
                host: host?.isEmpty == false ? host! : "127.0.0.1",
                port: Int(config.torExternalPort()),
                modeTitle: "Custom SOCKS5",
                quick: &quick
            )
        }

        applyWalletQuickStatus(to: &quick)
        quick.overall = overallTorQuickDot(quick)
        torQuickStatus = quick
    }

    @MainActor
    private func refreshBuiltInQuickStatus(quick: inout TorQuickStatus) async {
        do {
            _ = try await ensureBuiltInTorBootstrap()
        } catch {
            quick.torConnection = .red
            quick.torMessage = "Built-in Tor failed: \(error.localizedDescription)"
            quick.logs = recentTorLogs(torConnectionLogs())
            return
        }

        let logs = torConnectionLogs()
        let snapshot = deriveBuiltInBootstrapSnapshot(logs)
        let structuredStatus = builtInTorBootstrapStatus()
        let hasStructuredStatus = structuredStatus.launched
        quick.logs = recentTorLogs(logs)

        if structuredStatus.ready || (!hasStructuredStatus && snapshot.isReady) {
            quick.torConnection = .green
            quick.torMessage = "Built-in Tor ready"
        } else if let lastError = structuredStatus.lastError {
            quick.torConnection = .red
            quick.torMessage = lastError
        } else if !hasStructuredStatus, snapshot.hasError {
            quick.torConnection = .red
            quick.torMessage = snapshot.step
        } else {
            let message =
                structuredStatus.blocked.map { "Blocked: \($0)" }
                    ?? (leadingPercent(snapshot.step) != nil ? snapshot.step : nil)
                    ?? (hasStructuredStatus && !structuredStatus.message.isEmpty ? structuredStatus.message : snapshot.step)
            let percent = leadingPercent(message) ?? (hasStructuredStatus ? Int(structuredStatus.percent) : snapshot.percent)
            quick.torConnection = .yellow
            quick.torMessage = "Built-in Tor bootstrapping (\(percent)%)"
        }
    }

    @MainActor
    private func refreshProxyQuickStatus(
        host: String,
        port: Int,
        modeTitle: String,
        quick: inout TorQuickStatus
    ) async {
        let result = await testSocksEndpoint(host: host, port: port, timeout: 1.5)
        switch result {
        case .success:
            quick.torConnection = .green
            quick.torMessage = "\(modeTitle) proxy reachable at \(host):\(port)"
        case let .failure(error):
            quick.torConnection = .red
            quick.torMessage = "\(modeTitle) proxy unavailable: \(error.localizedDescription)"
        }
    }

    private func applyWalletQuickStatus(to quick: inout TorQuickStatus) {
        if quick.torConnection == .yellow {
            quick.nodeReachable = .yellow
            quick.nodeMessage = "Waiting for Tor"
            quick.nodeSynced = .yellow
            quick.syncMessage = "Waiting for Tor"
            return
        }

        if quick.torConnection == .red {
            quick.nodeReachable = .red
            quick.nodeMessage = "Tor unavailable"
            quick.nodeSynced = .red
            quick.syncMessage = "Tor unavailable"
            return
        }

        if case .nodeConnectionFailed = manager.errorAlert {
            quick.nodeReachable = .red
            quick.nodeMessage = "Node connection failed"
        } else {
            quick.nodeReachable = .green
            quick.nodeMessage = "Node reachable"
        }

        switch manager.loadState {
        case .loading:
            quick.nodeSynced = .yellow
            quick.syncMessage = "Wallet loading"
        case .scanning:
            quick.nodeSynced = .yellow
            quick.syncMessage = "Wallet syncing"
        case .loaded:
            quick.nodeSynced = .green
            quick.syncMessage = "Wallet synced"
        }
    }

    private func overallTorQuickDot(_ status: TorQuickStatus) -> TorStatusDot {
        let dots = [status.torConnection, status.nodeReachable, status.nodeSynced]
        if dots.allSatisfy({ $0 == .green }) { return .green }
        if dots.contains(.red) { return .red }
        if dots.contains(.yellow) { return .yellow }
        return .gray
    }

    private func recentTorLogs(_ logs: [String]) -> [String] {
        let usefulMarkers = [
            "arti_client::status",
            "tor_dirmgr",
            "tor_guardmgr",
            "tor_runtime",
            "bootstrapped",
            "bootstrap",
            "directory",
            "consensus",
            "microdescriptors",
            "failed",
            "error",
            "warn",
        ]
        let usefulLogs = logs
            .filter { line in
                usefulMarkers.contains { marker in
                    line.range(of: marker, options: .caseInsensitive) != nil
                }
            }
            .map { line in
                line.replacingOccurrences(
                    of: #"^\[(INFO|WARN|ERROR|DEBUG) [^\]]+]\s*"#,
                    with: "",
                    options: .regularExpression
                )
            }
            .filter { !$0.isEmpty }

        return Array(NSOrderedSet(array: usefulLogs).array.compactMap { $0 as? String }.suffix(6))
    }

    private func leadingPercent(_ message: String) -> Int? {
        guard let match = message.range(of: #"^\d{1,3}(?=%:)"#, options: .regularExpression) else {
            return nil
        }
        return Int(message[match]).map { min(max($0, 0), 100) }
    }

    var titleContent: some View {
        HStack(spacing: 10) {
            if case .cold = metadata.walletType {
                BitcoinShieldIcon(width: 13, color: toolbarTextColor)
            }

            Text(metadata.name)
                .foregroundStyle(toolbarTextColor)
                .font(.callout)
                .fontWeight(.semibold)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
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
                showRenameFromTitleMenu()
            }
        }
    }

    @ToolbarContentBuilder
    var MainToolBar: some ToolbarContent {
        ToolbarItemGroup(placement: .navigationBarTrailing) {
            HStack(spacing: 5) {
                if torQuickStatus.enabled {
                    Button(action: { showTorQuickStatus.toggle() }) {
                        HStack(spacing: 2) {
                            Image("iconTorOnion")
                                .renderingMode(.template)
                                .resizable()
                                .scaledToFit()
                                .frame(width: 26, height: 26)

                            BlinkingTorStatusDot(dot: torQuickStatus.overall, size: 10)
                        }
                        .adaptiveToolbarItemStyle(isPastHeader: shouldShowNavBar)
                    }
                    .popover(isPresented: $showTorQuickStatus) {
                        TorQuickStatusPopover(
                            status: torQuickStatus,
                            openNetworkSettings: {
                                showTorQuickStatus = false
                                app.pushRoute(.settings(.network))
                            }
                        )
                        .presentationCompactAdaptation(.popover)
                    }
                }

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
                        showExportLabelsConfirmation: $showExportLabelsConfirmation,
                        showExportXpubConfirmation: $showExportXpubConfirmation
                    )
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .adaptiveToolbarItemStyle(isPastHeader: shouldShowNavBar)
                        .font(.callout)
                }
                .accessibilityIdentifier("selectedWallet.more")
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
                .confirmationDialog(
                    "Export Xpub",
                    isPresented: $showExportXpubConfirmation
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

    var MainContent: some View {
        VStack(spacing: 0) {
            WalletBalanceHeaderView(
                balance: manager.balance.spendable(),
                metadata: manager.walletMetadata,
                updater: updater,
                showReceiveSheet: showReceiveSheet
            )
            .clipped()
            .onGeometryChange(for: CGFloat.self) { proxy in
                proxy.frame(in: .global).maxY
            } action: { _, headerBottom in
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

            if !cloudBackupManager.isConfigured {
                VerifyReminder(
                    walletId: manager.walletMetadata.id, isVerified: manager.walletMetadata.verified
                )
            }

            Transactions
                .environment(manager)
        }
        .background(Color.coveBg)
        .toolbar { MainToolBar }
        .navigationTitleView { titleContent }
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
                generateUrStrings: nil,
                copyData: { try await manager.rust.exportLabelsForShare().content }
            )
            .presentationDetents([.height(500), .height(600), .large])
            .padding()
            .padding(.top, 10)
        }
        .sheet(isPresented: $showXpubQrExport) {
            QrExportView(
                title: "Export Xpub",
                subtitle: "Public descriptor for\nwatch-only wallet",
                generateBbqrStrings: { density in
                    try await manager.rust.exportXpubForQr(density: density)
                },
                generateUrStrings: nil,
                copyData: { try await manager.rust.exportXpubForShare().content }
            )
            .presentationDetents([.height(500), .height(600), .large])
            .padding()
            .padding(.top, 10)
        }
        .onChange(of: scannedLabels, initial: false, onChangeOfScannedLabels)
        .task(id: manager.id) {
            await pollTorQuickStatus()
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
        ScrollViewReader { proxy in
            ScrollView {
                MainContent
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
                guard case .loaded = manager.loadState else { return }
                let task = Task.detached { try? await Task.sleep(for: .seconds(1.75)) }

                // wait for the task to complete
                let _ = await task.result
                runPostRefresh = true // mark for later
            }
            .task(id: runPostRefresh) {
                guard runPostRefresh else { return }
                defer { runPostRefresh = false }
                guard case let .loaded(txns) = manager.loadState else { return }

                self.manager.loadState = .scanning(txns)
                await manager.rust.forceWalletScan()
                let _ = try? await manager.rust.forceUpdateHeight()
                await manager.updateWalletBalance()
            }
            .onAppear {
                // Reset SendFlowManager so new send flow is fresh
                app.sendFlowManager = nil
                UIRefreshControl.appearance().tintColor = UIColor.white
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

private struct TorQuickStatusPopover: View {
    let status: TorQuickStatus
    let openNetworkSettings: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Tor Network Status")
                .font(.headline.weight(.bold))

            VStack(spacing: 12) {
                TorQuickStatusRow(
                    title: "Tor connection",
                    detail: status.torMessage,
                    dot: status.torConnection
                )
                TorQuickStatusRow(
                    title: "Node reachable",
                    detail: status.nodeMessage,
                    dot: status.nodeReachable
                )
                TorQuickStatusRow(
                    title: "Node synced",
                    detail: status.syncMessage,
                    dot: status.nodeSynced
                )
            }

            if !status.logs.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Recent logs")
                        .font(.caption.weight(.bold))
                        .foregroundStyle(.blue)

                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(status.logs.enumerated()), id: \.offset) { _, line in
                            Text(line)
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.midnightBlue.opacity(0.05))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                }
            }

            Button(action: openNetworkSettings) {
                Text("Network Settings")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.blue)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 4)
            }
        }
        .padding(18)
        .frame(width: 280)
    }
}

private struct TorQuickStatusRow: View {
    let title: String
    let detail: String
    let dot: TorStatusDot

    var body: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title.uppercased())
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(.secondary.opacity(0.8))

                Text(detail)
                    .font(.system(size: 13, weight: .semibold))
            }

            Spacer()

            if dot == .green {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(dot.color)
                    .font(.system(size: 16))
            } else {
                Circle()
                    .fill(dot.color)
                    .frame(width: 12, height: 12)
            }
        }
    }
}

private struct BlinkingTorStatusDot: View {
    let dot: TorStatusDot
    let size: CGFloat

    @State private var pulse = false

    var body: some View {
        Circle()
            .fill(dot.color)
            .frame(width: size, height: size)
            .opacity(dot == .yellow ? (pulse ? 1.0 : 0.28) : 1.0)
            .onAppear(perform: startPulseIfNeeded)
            .onChange(of: dot) { _, _ in
                startPulseIfNeeded()
            }
    }

    private func startPulseIfNeeded() {
        guard dot == .yellow else {
            pulse = false
            return
        }

        pulse = false
        withAnimation(.easeInOut(duration: 0.95).repeatForever(autoreverses: true)) {
            pulse = true
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
