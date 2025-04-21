//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

import CoveCore
import MijickPopupView
import SwiftUI

@_exported import CoveCore

struct NavigateKey: EnvironmentKey {
    static let defaultValue: (Route) -> Void = { _ in }
}

extension EnvironmentValues {
    var navigate: (Route) -> Void {
        get { self[NavigateKey.self] }
        set { self[NavigateKey.self] = newValue }
    }
}

struct SafeAreaInsetsKey: EnvironmentKey {
    static var defaultValue: EdgeInsets {
        #if os(iOS) || os(tvOS)
            let window = (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.keyWindow
            guard let insets = window?.safeAreaInsets else {
                return EdgeInsets()
            }
            return EdgeInsets(
                top: insets.top, leading: insets.left, bottom: insets.bottom, trailing: insets.right
            )
        #else
            return EdgeInsets()
        #endif
    }
}

public extension EnvironmentValues {
    var safeAreaInsets: EdgeInsets {
        self[SafeAreaInsetsKey.self]
    }
}

@main
struct CoveApp: App {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.scenePhase) private var phase

    @State var app: AppManager
    @State var auth: AuthManager

    @State var id = UUID()

    @State var showCover: Bool = true
    @State var scannedCode: TaggedItem<StringOrData>? = .none
    @State var coverClearTask: Task<Void, Never>?

    @ViewBuilder
    private func alertMessage(alert: TaggedItem<AppAlertState>) -> some View {
        let text =
            switch alert.item {
            case .invalidWordGroup:
                "The words from the file does not create a valid wallet. Please check the words and try again."
            case .duplicateWallet:
                "This wallet has already been imported! Taking you there now..."
            case .errorImportingHotWallet:
                "Error Importing Wallet"
            case .importedSuccessfully:
                "Wallet Imported Successfully"
            case .importedLabelsSuccessfully:
                "Labels Imported Successfully"
            case .unableToSelectWallet:
                "Unable to select wallet, please try again"
            case let .errorImportingHardwareWallet(error):
                "Error: \(error)"
            case .invalidFileFormat:
                "The file or scanned code did not match any formats that Cove supports."
            case let .invalidFormat(error):
                error
            case let .addressWrongNetwork(
                address: address, network: network, currentNetwork: currentNetwork
            ):
                "The address \(address) is on the wrong network. You are on \(currentNetwork), and the address was for \(network)."
            case let .noWalletSelected(address),
                 let .foundAddress(address, _):
                address.unformatted()
            case .noCameraPermission:
                "Please allow camera access in Settings to use this feature."
            case let .failedToScanQr(error):
                "Error: \(error)"
            case let .noUnsignedTransactionFound(txId):
                "No unsigned transaction found for transaction \(txId.asHashString())"
            case let .unableToGetAddress(error: error):
                "Error: \(error)"
            case .cantSendOnWatchOnlyWallet:
                "This is watch-only wallet and cannot send transactions. Please import this wallet again to enable sending transactions."
            case .uninitializedTapSigner:
                "This TAPSIGNER has not been setup yet. Would you like to setup it now?"
            case let .tapSignerSetupFailed(error):
                "Please try again.\(error)"
            case let .tapSignerDeriveFailed(error):
                "Please try again.\nError: \(error)"
            case .tapSignerInvalidAuth:
                "The PIN you entered was incorrect. Please try again."
            case .intializedTapSigner:
                "Would you like to start using this TAPSIGNER with Cove?"
            case .tapSignerWalletFound:
                "Would you like to go to this wallet?"
            case let .tapSignerNoBackup(tapSigner):
                "Can't change the PIN without taking a backup of the wallet. Would you like to take a backup now?"
            case .general(title: _, let message):
                message
            }

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
        case let .duplicateWallet(walletId):
            Button("OK") {
                app.alertState = .none
                app.isSidebarVisible = false
                try? app.rust.selectWallet(id: walletId)
            }
        case let .addressWrongNetwork(address: address, network: _, currentNetwork: _):
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
        case let .foundAddress(address, amount):
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
        case let .tapSignerWalletFound(id):
            Button("Yes") { app.selectWallet(id) }
            Button("Cancel", role: .cancel) { app.alertState = .none }
        case let .intializedTapSigner(t):
            Button("Yes") {
                app.sheetState = .init(
                    .tapSigner(
                        .enterPin(tapSigner: t, action: .derive)
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
             .cantSendOnWatchOnlyWallet,
             .tapSignerSetupFailed,
             .tapSignerInvalidAuth,
             .tapSignerDeriveFailed,
             .general,
             .invalidFormat:
            Button("OK") {
                app.alertState = .none
            }
        }
    }

    public init() {
        // initialize keychain and device
        _ = Keychain(keychain: KeychainAccessor())
        _ = Device(device: DeviceAccesor())

        let app = AppManager.shared
        let auth = AuthManager.shared

        self.app = app
        self.auth = auth
    }

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { app.alertState != nil },
            set: { newValue in
                if !newValue {
                    app.alertState = .none
                }
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

    @MainActor
    func importHotWallet(_ words: [String]) {
        do {
            let manager = ImportWalletManager()
            let walletMetadata = try manager.rust.importWallet(enteredWords: [words])
            try app.rust.selectWallet(id: walletMetadata.id)
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                app.alertState = TaggedItem(.invalidWordGroup)
            case let .WalletAlreadyExists(walletId):
                app.alertState = TaggedItem(.duplicateWallet(walletId))
            default:
                Log.error("Unable to import wallet: \(error)")
                app.alertState = TaggedItem(
                    .errorImportingHotWallet(error.localizedDescription))
            }
        } catch {
            Log.error("Unknown error \(error)")
            app.alertState = TaggedItem(
                .errorImportingHotWallet(error.localizedDescription))
        }
    }

    func importColdWallet(_ export: HardwareExport) {
        do {
            let wallet = try Wallet.newFromExport(export: export)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            app.alertState = TaggedItem(.importedSuccessfully)
            try app.rust.selectWallet(id: id)
        } catch let WalletError.WalletAlreadyExists(id) {
            app.alertState = TaggedItem(.duplicateWallet(id))

            if (try? app.rust.selectWallet(id: id)) == nil {
                app.alertState = TaggedItem(.unableToSelectWallet)
            }
        } catch {
            app.alertState = TaggedItem(
                .errorImportingHardwareWallet(error.localizedDescription))
        }
    }

    func handleAddress(_ addressWithNetwork: AddressWithNetwork) {
        let currentNetwork = Database().globalConfig().selectedNetwork()
        let address = addressWithNetwork.address()
        let network = addressWithNetwork.network()
        let selectedWallet = Database().globalConfig().selectedWallet()

        if selectedWallet == nil {
            app.alertState = TaggedItem(AppAlertState.noWalletSelected(address))
            return
        }

        if network != currentNetwork, network == .bitcoin || currentNetwork == .bitcoin {
            app.alertState = TaggedItem(
                AppAlertState.addressWrongNetwork(
                    address: address, network: network, currentNetwork: currentNetwork
                ))
            return
        }

        let amount = addressWithNetwork.amount()
        app.alertState = TaggedItem(.foundAddress(address, amount))
    }

    func handleTransaction(_ transaction: BitcoinTransaction) {
        Log.debug(
            "Received BitcoinTransaction: \(transaction): \(transaction.txIdHash())"
        )

        let db = Database().unsignedTransactions()
        let txnRecord = db.getTx(txId: transaction.txId())

        guard let txnRecord else {
            Log.error("No unsigned transaction found for \(transaction.txId())")
            app.alertState = .init(.noUnsignedTransactionFound(transaction.txId()))
            return
        }

        let route = RouteFactory().sendConfirm(
            id: txnRecord.walletId(), details: txnRecord.confirmDetails(),
            signedTransaction: transaction
        )

        app.pushRoute(route)
    }

    func handleFileOpen(_ url: URL) {
        let fileHandler = FileHandler(filePath: url.absoluteString)

        do {
            let readResult = try fileHandler.read()
            switch readResult {
            case let .mnemonic(mnemonic):
                importHotWallet(mnemonic.words())
            case let .hardwareExport(export):
                importColdWallet(export)
            case let .address(addressWithNetwork):
                handleAddress(addressWithNetwork)
            case let .transaction(txn):
                handleTransaction(txn)
            case let .tapSignerInit(tapSigner):
                app.sheetState = .init(.tapSigner(TapSignerRoute.initSelect(tapSigner)))
            case let .tapSigner(tapSigner):
                let panic =
                    "TAPSIGNER not implemented \(tapSigner) doesn't make sense for file import"
                Log.error(panic)
            case let .bip329Labels(labels):
                if let selectedWallet = Database().globalConfig().selectedWallet() {
                    return try LabelManager(id: selectedWallet).import(labels: labels)
                }

                app.alertState = TaggedItem(
                    .invalidFileFormat(
                        "Currently BIP329 labels must be imported through the wallet actions"))
            }
        } catch {
            switch error {
            case let FileHandlerError.NotRecognizedFormat(multiFormatError):
                Log.error("Unrecognized format mulit format error: \(multiFormatError)")
                app.alertState = TaggedItem(
                    .invalidFileFormat(multiFormatError.localizedDescription))

            case let FileHandlerError.OpenFile(error):
                Log.error("File handler error: \(error)")

            case let FileHandlerError.ReadFile(error):
                Log.error("Unable to read file: \(error)")

            case FileHandlerError.FileNotFound:
                Log.error("File not found")

            default:
                Log.error("Unknown error file handling file: \(error)")
            }
        }
    }

    func setInvalidlabels() {
        app.alertState = TaggedItem(
            .invalidFileFormat(
                "Currently BIP329 labels must be imported through the wallet actions"))
    }

    @MainActor
    func handleMultiFormat(_ multiFormat: MultiFormat) {
        do {
            switch multiFormat {
            case let .mnemonic(mnemonic):
                importHotWallet(mnemonic.words())
            case let .hardwareExport(export):
                importColdWallet(export)
            case let .address(addressWithNetwork):
                handleAddress(addressWithNetwork)
            case let .transaction(transaction):
                handleTransaction(transaction)
            case let .tapSignerInit(tapSigner):
                app.alertState = .init(.uninitializedTapSigner(tapSigner))
            case let .tapSigner(tapSigner):
                if let wallet = app.findTapSignerWallet(tapSigner) {
                    app.alertState = .init(.tapSignerWalletFound(wallet.id))
                } else {
                    app.alertState = .init(.intializedTapSigner(tapSigner))
                }
            case let .bip329Labels(labels):
                guard let manager = app.walletManager else { return setInvalidlabels() }
                guard let selectedWallet = Database().globalConfig().selectedWallet() else {
                    return setInvalidlabels()
                }

                // import the labels
                try LabelManager(id: selectedWallet).import(labels: labels)
                app.alertState = .init(.importedLabelsSuccessfully)

                // when labels are imported, we need to get the transactions again with the updated labels
                Task { await manager.rust.getTransactions() }
            }
        } catch {
            switch error {
            case let multiFormatError as MultiFormatError:
                Log.error(
                    "MultiFormat not recognized: \(multiFormatError): \(multiFormatError.describe)"
                )
                app.alertState = TaggedItem(.invalidFormat(multiFormatError.describe))

            default:
                Log.error("Unable to handle scanned code, error: \(error)")
                app.alertState = TaggedItem(.invalidFileFormat(error.localizedDescription))
            }
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

    @ViewBuilder
    var BodyView: some View {
        Group {
            if showCover {
                CoverView()
            } else if app.isLoading {
                FullPageLoadingView()
            } else {
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
                                                .foregroundStyle(navBarColor)
                                        }
                                        .contentShape(Rectangle())
                                        .foregroundStyle(navBarColor)
                                    }
                                }
                        }
                        .tint(routeToTint)
                    }
                }
            }
        }
        .onChange(of: auth.lockState) { old, new in
            Log.warn("AUTH LOCK STATE CHANGED: \(old) --> \(new)")
        }
        .environment(app)
        .environment(auth)
    }

    var routeToTint: Color {
        switch app.router.routes.last {
        case .settings, .transactionDetails:
            .blue
        default:
            .white
        }
    }

    func onChangeRoute(_ old: [Route], _ new: [Route]) {
        if !old.isEmpty, new.isEmpty { id = UUID() }

        app.dispatch(action: AppAction.updateRoute(routes: new))
    }

    func onChangeQr(
        _: TaggedItem<StringOrData>?, _ scannedCode: TaggedItem<StringOrData>?
    ) {
        Log.debug("[COVE APP ROOT] onChangeQr")
        guard let scannedCode else { return }
        app.sheetState = .none
        do {
            let multiFormat = try scannedCode.item.toMultiFormat()
            handleMultiFormat(multiFormat)
        } catch {
            switch error {
            case let multiFormatError as MultiFormatError:
                Log.error(
                    "MultiFormat not recognized: \(multiFormatError): \(multiFormatError.describe)"
                )
                app.alertState = TaggedItem(.invalidFormat(multiFormatError.describe))

            default:
                Log.error("Unable to handle scanned code, error: \(error)")
                app.alertState = TaggedItem(.invalidFileFormat(error.localizedDescription))
            }
        }
    }

    func onChangeNfc(_: NfcMessage?, _ nfcMessage: NfcMessage?) {
        Log.debug("[COVE APP ROOT] onChangeNfc")
        guard let nfcMessage else { return }
        do {
            let multiFormat = try nfcMessage.tryIntoMultiFormat()
            handleMultiFormat(multiFormat)
        } catch {
            switch error {
            case let multiFormatError as MultiFormatError:
                Log.error(
                    "MultiFormat not recognized: \(multiFormatError): \(multiFormatError.describe)"
                )
                app.alertState = TaggedItem(.invalidFormat(multiFormatError.describe))

            default:
                Log.error("Unable to handle scanned code, error: \(error)")
                app.alertState = TaggedItem(.invalidFileFormat(error.localizedDescription))
            }
        }
    }

    func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        Log.debug(
            "[SCENE PHASE]: \(oldPhase) --> \(newPhase) && using biometrics: \(auth.isUsingBiometrics)"
        )

        if !auth.isAuthEnabled {
            showCover = false
            auth.unlock()
        }

        if newPhase == .active { showCover = false }

        // PIN auth active, no biometrics, leaving app
        if auth.isAuthEnabled,
           !auth.isUsingBiometrics,
           oldPhase == .active,
           newPhase == .inactive
        {
            Log.debug("[scene] app going inactive")
            coverClearTask?.cancel()

            if !app.nfcWriter.isScanning, !app.nfcReader.isScanning { showCover = true }

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

        // close all open sheets when going into the background
        if auth.isAuthEnabled, newPhase == .background {
            Log.debug("[scene] app going into background")
            coverClearTask?.cancel()

            showCover = true
            if auth.lockState != .locked { auth.lock() }

            UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
                .forEach { window in
                    window.rootViewController?.dismiss(animated: false)
                }
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

    var body: some Scene {
        WindowGroup {
            BodyView
                .implementPopupView()
                .id(id)
                .environment(\.navigate) { route in
                    app.pushRoute(route)
                }
                .environment(app)
                .preferredColorScheme(app.colorScheme)
                .onChange(of: app.router.routes, onChangeRoute)
                .onChange(of: app.selectedNetwork) { id = UUID() }
                // QR code scanning
                .onChange(of: scannedCode, onChangeQr)
                // NFC scanning
                .onChange(of: app.nfcReader.scannedMessage, onChangeNfc)
                .alert(
                    app.alertState?.item.title() ?? "Alert",
                    isPresented: showingAlert,
                    presenting: app.alertState,
                    actions: alertButtons,
                    message: alertMessage
                )
                .sheet(item: $app.sheetState, content: SheetContent)
                .gesture(
                    app.router.routes.isEmpty
                        ? DragGesture()
                        .onChanged { gesture in
                            if gesture.startLocation.x < 25, gesture.translation.width > 100 {
                                withAnimation(.spring()) {
                                    app.isSidebarVisible = true
                                }
                            }
                        }
                        .onEnded { gesture in
                            if gesture.startLocation.x < 20, gesture.translation.width > 50 {
                                withAnimation(.spring()) {
                                    app.isSidebarVisible = true
                                }
                            }
                        } : nil
                )
                .task {
                    await app.rust.initOnStart()
                    await MainActor.run { app.asyncRuntimeReady = true }
                }
                .onOpenURL(perform: handleFileOpen)
                .onChange(of: phase, initial: true, handleScenePhaseChange)
        }
    }
}
