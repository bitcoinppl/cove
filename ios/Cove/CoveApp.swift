//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

import MijickPopupView
import SwiftUI

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
    @AppStorage("lockedAt") var lockedAt: Date = .init()

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
                String(address)
            case .noCameraPermission:
                "Please allow camera access in Settings to use this feature."
            case let .failedToScanQr(error):
                "Error: \(error)"
            case let .noUnsignedTransactionFound(txId):
                "No unsigned transaction found for transaction \(txId.asHashString())"
            case let .unableToGetAddress(error: error):
                "Error: \(error)"
            }

        Text(text)
    }

    @ViewBuilder
    private func alertButtons(alert: TaggedItem<AppAlertState>) -> some View {
        switch alert.item {
        case let .duplicateWallet(walletId):
            Button("OK") {
                app.alertState = .none
                try? app.rust.selectWallet(id: walletId)
            }
        case .invalidWordGroup,
             .errorImportingHotWallet,
             .importedSuccessfully,
             .unableToSelectWallet,
             .errorImportingHardwareWallet,
             .invalidFileFormat,
             .invalidFormat:
            Button("OK") {
                app.alertState = .none
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
        case .failedToScanQr, .noUnsignedTransactionFound:
            Button("OK") { app.alertState = .none }
        default:
            Button("OK") { app.alertState = .none }
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
            case let .bip329Labels(labels):
                if let selectedWallet = Database().globalConfig().selectedWallet() {
                    return try LabelManager(id: selectedWallet).import(labels: labels)
                }

                app.alertState = TaggedItem(.invalidFileFormat("Currently BIP329 labels must be imported through the wallet actions"))
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

    @MainActor
    func handleScannedCode(_ stringOrData: StringOrData) {
        do {
            let multiFormat = try stringOrData.toMultiFormat()
            switch multiFormat {
            case let .mnemonic(mnemonic):
                importHotWallet(mnemonic.words())
            case let .hardwareExport(export):
                importColdWallet(export)
            case let .address(addressWithNetwork):
                handleAddress(addressWithNetwork)
            case let .transaction(transaction):
                handleTransaction(transaction)
            case let .bip329Labels(labels):
                if let selectedWallet = Database().globalConfig().selectedWallet() {
                    return try LabelManager(id: selectedWallet).import(labels: labels)
                }
                app.alertState = TaggedItem(.invalidFileFormat("Currently BIP329 labels must be imported through the wallet actions"))
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
        if !old.isEmpty, new.isEmpty {
            id = UUID()
        }

        app.dispatch(action: AppAction.updateRoute(routes: new))
    }

    func onChangeQr(
        _: TaggedItem<StringOrData>?, _ scannedCode: TaggedItem<StringOrData>?
    ) {
        guard let scannedCode else { return }
        app.sheetState = .none
        handleScannedCode(scannedCode.item)
    }

    func onChangeNfc(_: String?, _ scannedMessage: String?) {
        guard let scannedMessage else { return }
        if scannedMessage.isEmpty { return }
        handleScannedCode(StringOrData(scannedMessage))
    }

    func onChangeNfcData(_: Data?, _ scannedMessage: Data?) {
        guard let scannedMessage else { return }
        if scannedMessage.isEmpty { return }
        handleScannedCode(StringOrData(scannedMessage))
    }

    func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        Log.debug(
            "[SCENE PHASE]: \(oldPhase) --> \(newPhase) && using biometrics: \(auth.isUsingBiometrics)"
        )

        if !auth.isAuthEnabled { showCover = false }
        if newPhase == .active { showCover = false }

        // PIN auth active, no biometrics, leaving app
        if auth.isAuthEnabled,
           !auth.isUsingBiometrics,
           oldPhase == .active,
           newPhase == .inactive
        {
            coverClearTask?.cancel()
            showCover = true

            // prevent getting stuck on show cover
            coverClearTask = Task {
                try? await Task.sleep(for: .milliseconds(200))
                if phase == .active { showCover = false }
            }

            if auth.lockState != .locked {
                auth.lockState = .locked
                lockedAt = Date.now
            }
        }

        // close all open sheets when going into the background
        if auth.isAuthEnabled, oldPhase == .inactive, newPhase == .background {
            coverClearTask?.cancel()

            if auth.lockState != .locked {
                auth.lockState = .locked
                lockedAt = Date.now
            }

            UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
                .forEach { window in
                    window.rootViewController?.dismiss(animated: false)
                }
        }

        // auth enabled, opening app again
        if auth.isAuthEnabled, oldPhase == .inactive, newPhase == .active {
            let sinceLocked = Date.now.timeIntervalSince(lockedAt)
            Log.debug("LOCKED AT: \(lockedAt) == \(sinceLocked)")

            // less than 3 seconds, auto unlock if PIN only, and not in decoy mode
            // TODO: make this configurable and put in DB
            if auth.type == .pin, !auth.isInDecoyMode(), sinceLocked < 3 {
                showCover = false
                auth.lockState = .unlocked
            }

            // auto unlock if its less than a second for other lock type
            if !auth.isInDecoyMode(), sinceLocked < 1 {
                showCover = false
                auth.lockState = .unlocked
            }
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
                .onChange(of: app.nfcReader.scannedMessageData, onChangeNfcData)
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
                .onAppear {
                    if auth.isAuthEnabled {
                        auth.lockState = .locked
                    } else {
                        auth.lockState = .unlocked
                    }
                }
        }
    }
}
