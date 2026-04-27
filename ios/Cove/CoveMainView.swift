//
//  CoveMainView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct CoveMainView: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.scenePhase) private var phase

    @State var app: AppManager
    @State var auth: AuthManager

    @State var id = UUID()
    @State var showCover: Bool = true
    @State var scannedCode: TaggedItem<MultiFormat>? = .none
    @State var coverClearTask: Task<Void, Never>?

    @ViewBuilder
    private func alertMessage(alert: TaggedItem<AppAlertState>) -> some View {
        let text = alert.item.message()

        if case .foundAddress = alert.item {
            Text(text.map { "\($0)\u{200B}" }.joined())
                .font(.system(.caption2, design: .monospaced))
                .minimumScaleFactor(0.5)
                .lineLimit(2)
        } else {
            Text(text)
        }
    }

    @ViewBuilder
    private func alertButtons(alert: TaggedItem<AppAlertState>) -> some View {
        switch alert.item {
        case let .duplicateWallet(walletId: walletId):
            Button("OK") {
                app.alertState = .none
                app.isSidebarVisible = false
                try? app.rust.selectWallet(id: walletId)
            }
        case let .hotWalletKeyMissing(walletId: walletId):
            if CloudBackupManager.shared.isCloudBackupEnabled {
                Button("Open Cloud Backup") {
                    app.alertState = .none
                    app.loadAndReset(to: .settings(.cloudBackup))
                }
            }

            Button("Import 12 Words") {
                app.alertState = .none
                app.loadAndReset(to: .newWallet(.hotWallet(.import(.twelve, .manual))))
            }

            Button("Import 24 Words") {
                app.alertState = .none
                app.loadAndReset(to: .newWallet(.hotWallet(.import(.twentyFour, .manual))))
            }

            Button("Use with Hardware Wallet") {
                do {
                    try app.getWalletManager(id: walletId).rust.setWalletType(walletType: .cold)
                    app.alertState = .none
                } catch {
                    Log.error("Failed to set wallet type to cold: \(error)")
                    DispatchQueue.main.async {
                        app.alertState = .init(
                            .general(
                                title: "Error",
                                message: error.localizedDescription
                            )
                        )
                    }
                }
            }

            Button("Use as Watch Only", role: .cancel) {
                DispatchQueue.main.async { app.alertState = .init(.confirmWatchOnly) }
            }
        case .confirmWatchOnly:
            Button("I Understand", role: .destructive) {
                app.alertState = .none
            }
        case let .addressWrongNetwork(address, _, _):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                app.alertState = .none
            }
        case let .noWalletSelected(address):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                app.alertState = .none
            }
        case let .foundAddress(address: address, amount: amount):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            if let id = Database().globalConfig().selectedWallet() {
                Button("Send To Address") {
                    let route = RouteFactory().sendSetAmount(
                        id: id, address: address, amount: amount
                    )
                    app.pushRoute(route)
                    app.alertState = .none
                }
            }

            Button("Cancel") {
                app.alertState = .none
            }
        case .noCameraPermission:
            Button("OK") {
                app.alertState = .none
                let url = URL(string: UIApplication.openSettingsURLString)!
                UIApplication.shared.open(url)
            }
        case let .uninitializedTapSigner(tapSigner):
            Button("Yes") {
                app.isSidebarVisible = false
                app.sheetState = .init(.tapSigner(TapSignerRoute.initSelect(tapSigner)))
            }

            Button("Cancel", role: .cancel) {
                app.alertState = .none
            }
        case let .tapSignerWalletFound(walletId):
            Button("Yes") { app.selectWallet(walletId) }
            Button("Cancel", role: .cancel) { app.alertState = .none }
        case let .initializedTapSigner(tapSigner):
            Button("Yes") {
                app.sheetState = .init(
                    .tapSigner(
                        .enterPin(tapSigner: tapSigner, action: .derive)
                    )
                )
            }
            Button("Cancel", role: .cancel) { app.alertState = .none }
        case let .tapSignerNoBackup(tapSigner):
            Button("Yes") {
                print("TODO: go to backup screen \(tapSigner)}")
                // TODO: go to backup screen
            }
            Button("Cancel", role: .cancel) { app.alertState = .none }
        case let .tapSignerWrongPin(tapSigner, action):
            Button("Try Again") {
                app.sheetState = .init(.tapSigner(.enterPin(tapSigner: tapSigner, action: action)))
            }
            Button("Cancel", role: .cancel) { app.alertState = .none }
        case .cantSendOnWatchOnlyWallet:
            Button("Import Hardware Wallet") {
                DispatchQueue.main.async { app.alertState = .init(.watchOnlyImportHardware) }
            }
            Button("Import Words") {
                DispatchQueue.main.async { app.alertState = .init(.watchOnlyImportWords) }
            }
            Button("Cancel", role: .cancel) {
                app.alertState = .none
            }
        case .watchOnlyImportHardware:
            Button("QR Code") {
                app.alertState = .none
                app.pushRoute(.newWallet(.coldWallet(.qrCode)))
            }
            Button("NFC") {
                app.alertState = .none
                app.nfcReader.scan()
            }
            Button("Paste") {
                app.alertState = .none
                let text = UIPasteboard.general.string ?? ""
                if text.isEmpty { return }
                do {
                    let wallet = try Wallet.newFromXpub(xpub: text)
                    try app.rust.selectWallet(id: wallet.id())
                    app.resetRoute(to: .selectedWallet(wallet.id()))
                } catch {
                    DispatchQueue.main.async {
                        app.alertState = .init(
                            .errorImportingHardwareWallet(message: error.localizedDescription)
                        )
                    }
                }
            }
            Button("Cancel", role: .cancel) {
                app.alertState = .none
            }
        case .watchOnlyImportWords:
            Button("Scan QR") {
                app.alertState = .none
                app.pushRoute(.newWallet(.hotWallet(.import(.twentyFour, .qr))))
            }
            Button("NFC") {
                app.alertState = .none
                app.pushRoute(.newWallet(.hotWallet(.import(.twentyFour, .nfc))))
            }
            Button("12 Words") {
                app.alertState = .none
                app.pushRoute(.newWallet(.hotWallet(.import(.twelve, .manual))))
            }
            Button("24 Words") {
                app.alertState = .none
                app.pushRoute(.newWallet(.hotWallet(.import(.twentyFour, .manual))))
            }
            Button("Cancel", role: .cancel) {
                app.alertState = .none
            }
        case let .walletDatabaseCorrupted(walletId, _):
            Button("Delete Wallet", role: .destructive) {
                app.alertState = .none
                app.rust.deleteCorruptedWallet(id: walletId)
            }
            Button("Cancel", role: .cancel) {
                app.alertState = .none
                app.rust.selectLatestOrNewWallet()
            }
        case .invalidWordGroup,
             .errorImportingHotWallet,
             .importedSuccessfully,
             .unableToSelectWallet,
             .errorImportingHardwareWallet,
             .invalidFileFormat,
             .importedLabelsSuccessfully,
             .unableToGetAddress,
             .failedToScanQr,
             .noUnsignedTransactionFound,
             .tapSignerSetupFailed,
             .tapSignerInvalidAuth,
             .tapSignerDeriveFailed,
             .general,
             .invalidFormat,
             .loading:
            Button("OK") {
                app.alertState = .none
            }
        }
    }

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { app.alertState != nil },
            set: { newValue in
                if !newValue { app.alertState = .none }
            }
        )
    }

    var navBarColor: Color {
        switch app.currentRoute {
        case .newWallet(.hotWallet(.create)):
            Color.white
        case .newWallet(.hotWallet(.verifyWords)):
            Color.white
        case .selectedWallet:
            Color.white
        default:
            Color.blue
        }
    }

    @ViewBuilder
    func SheetContent(_ state: TaggedItem<AppSheetState>) -> some View {
        switch state.item {
        case .qr:
            QrCodeScanView(app: app, scannedCode: $scannedCode)
        case let .tapSigner(route):
            TapSignerContainer(route: route)
                .environment(app)
        }
    }

    var BodyView: some View {
        Group {
            LockView(
                lockType: auth.type,
                isPinCorrect: { pin in
                    auth.handleAndReturnUnlockMode(pin) != .locked
                },
                showPin: false,
                lockState: $auth.lockState,
                onUnlock: { _ in
                    withAnimation { showCover = false }
                }
            ) {
                SidebarContainer {
                    NavigationStack(path: $app.router.routes) {
                        RouteView(app: app)
                            .navigationDestination(
                                for: Route.self,
                                destination: { route in
                                    RouteView(app: app, route: route)
                                }
                            )
                            .toolbar {
                                ToolbarItem(placement: .navigationBarLeading) {
                                    Button(action: {
                                        withAnimation {
                                            app.toggleSidebar()
                                        }
                                    }) {
                                        Image(systemName: "line.horizontal.3")
                                            .modifier(
                                                NavBarColorModifier(
                                                    route: app.currentRoute,
                                                    isPastHeader: app.isPastHeader
                                                )
                                            )
                                    }
                                    .contentShape(Rectangle())
                                }
                            }
                    }
                    .modifier(ConditionalRouteTintModifier(route: app.router.routes.last))
                }
            }
            .fullScreenCover(isPresented: $app.isLoading) {
                FullPageLoadingView().interactiveDismissDisabled(true)
            }
            .fullScreenCover(isPresented: $showCover) {
                CoverView().interactiveDismissDisabled(true)
            }
        }
        .onChange(of: auth.lockState) { old, new in
            Log.warn("AUTH LOCK STATE CHANGED: \(old) --> \(new)")
        }
        .environment(app)
        .environment(auth)
    }

    func onChangeRoute(_ old: [Route], _ new: [Route]) {
        // defer view identity reset to avoid ChildEnvironment propagation loop
        // during UIKit's parallax transition animation
        if !old.isEmpty, new.isEmpty {
            DispatchQueue.main.async { id = UUID() }
        }

        app.dispatch(action: AppAction.updateRoute(routes: new))
    }

    func onChangeQr(
        _: TaggedItem<MultiFormat>?, _ scannedCode: TaggedItem<MultiFormat>?
    ) {
        Log.debug("[COVE APP ROOT] onChangeQr")
        guard let scannedCode else { return }
        app.sheetState = .none
        ScanManager.shared.handleMultiFormat(scannedCode.item)
    }

    func onChangeNfc(_: NfcMessage?, _ nfcMessage: NfcMessage?) {
        Log.debug("[COVE APP ROOT] onChangeNfc")
        guard let nfcMessage else { return }
        ScanManager.shared.handleNfcScan(nfcMessage)
    }

    func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        Log.debug(
            "[SCENE PHASE]: \(oldPhase) --> \(newPhase) && using biometrics: \(auth.isUsingBiometrics)"
        )

        if !auth.isAuthEnabled {
            showCover = false
            auth.unlock()
        }

        if newPhase == .active {
            showCover = false
            guard app.asyncRuntimeReady else { return }
            app.dispatch(action: AppAction.updateFees)
            app.dispatch(action: AppAction.updateFiatPrices)
        }

        // PIN auth active, no biometrics, leaving app
        if auth.isAuthEnabled,
           !auth.isUsingBiometrics,
           oldPhase == .active,
           newPhase == .inactive
        {
            Log.debug("[scene] app going inactive")
            coverClearTask?.cancel()

            let tapSignerScanning = app.tapSignerNfc?.isScanning ?? false
            if !app.nfcWriter.isScanning, !app.nfcReader.isScanning, !tapSignerScanning {
                showCover = true
            }

            // prevent getting stuck on show cover
            coverClearTask = Task {
                try? await Task.sleep(for: .milliseconds(100))
                if Task.isCancelled { return }

                if phase == .active { showCover = false }

                try? await Task.sleep(for: .milliseconds(200))
                if Task.isCancelled { return }

                if phase == .active { showCover = false }
            }
        }

        if newPhase == .background { app.isSidebarVisible = false }

        // close all open sheets when going into the background
        if auth.isAuthEnabled, newPhase == .background {
            Log.debug("[scene] app going into background")
            coverClearTask?.cancel()

            // don't lock or dismiss sheets if any NFC operation is active
            let tapSignerScanning = app.tapSignerNfc?.isScanning ?? false
            if app.nfcWriter.isScanning || app.nfcReader.isScanning || tapSignerScanning {
                Log.debug("[scene] NFC operation active, not dismissing sheets or locking")
                return
            }

            showCover = true
            if auth.lockState != .locked { auth.lock() }

            UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
                .forEach { window in
                    window.rootViewController?.dismiss(animated: false)
                }

            // dismiss all keyboard
            UIApplication.shared.endEditing()
        }

        // auth enabled, opening app again
        if auth.isAuthEnabled, oldPhase == .inactive, newPhase == .active {
            guard let lockedAt = auth.lockedAt else { return }
            let sinceLocked = Date.now.timeIntervalSince(lockedAt)
            Log.debug("[ROOT][AUTH] lockedAt \(lockedAt) == \(sinceLocked)")

            // less than 1 second, auto unlock if PIN only, and not in decoy mode
            // TODO: make this configurable and put in DB
            if auth.type == .pin, !auth.isDecoyPinEnabled, sinceLocked < 2 {
                showCover = false
                auth.unlock()
                return
            }

            if sinceLocked < 1 {
                showCover = false
                auth.unlock()
            }
        }

        // sanity check, get out of decoy mode if PIN is disabled
        if auth.isInDecoyMode(), newPhase == .active,
           auth.type == .none || auth.type == .biometric
        {
            auth.switchToMainMode()
        }
    }

    var body: some View {
        CloudBackupPresentationHost(app: app, auth: auth, isCoverPresented: showCover) {
            BodyView
                .id(id)
                .environment(\.navigate) { route in
                    app.pushRoute(route)
                }
                .environment(app)
                .preferredColorScheme(app.colorScheme)
                .onChange(of: app.router.routes, onChangeRoute)
                .onChange(of: app.selectedNetwork) { id = UUID() }
                .onChange(of: scannedCode, onChangeQr)
                .onChange(of: app.nfcReader.scannedMessage, onChangeNfc)
                .alert(
                    app.alertState?.item.title() ?? "Alert",
                    isPresented: showingAlert,
                    presenting: app.alertState,
                    actions: alertButtons,
                    message: alertMessage
                )
                .sheet(item: $app.sheetState, content: SheetContent)
                .onOpenURL(perform: ScanManager.shared.handleFileOpen)
                .onChange(of: phase, initial: true, handleScenePhaseChange)
        }
    }
}
