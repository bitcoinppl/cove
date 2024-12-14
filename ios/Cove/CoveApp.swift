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

    @State var manager: AppManager
    @State var id = UUID()

    @State var scannedCode: TaggedItem<StringOrData>? = .none

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
            }

        Text(text)
    }

    @ViewBuilder
    private func alertButtons(alert: TaggedItem<AppAlertState>) -> some View {
        switch alert.item {
        case let .duplicateWallet(walletId):
            Button("OK") {
                manager.alertState = .none
                try? manager.rust.selectWallet(id: walletId)
            }
        case .invalidWordGroup,
             .errorImportingHotWallet,
             .importedSuccessfully,
             .unableToSelectWallet,
             .errorImportingHardwareWallet,
             .invalidFileFormat:
            Button("OK") {
                manager.alertState = .none
            }
        case let .addressWrongNetwork(address: address, network: _, currentNetwork: _):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                manager.alertState = .none
            }
        case let .noWalletSelected(address):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                manager.alertState = .none
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
                    manager.pushRoute(route)
                    manager.alertState = .none
                }
            }

            Button("Cancel") {
                manager.alertState = .none
            }
        case .noCameraPermission:
            Button("OK") {
                manager.alertState = .none
                let url = URL(string: UIApplication.openSettingsURLString)!
                UIApplication.shared.open(url)
            }
        case .failedToScanQr, .noUnsignedTransactionFound:
            Button("OK") {
                manager.alertState = .none
            }
        }
    }

    public init() {
        // initialize keychain and device
        _ = Keychain(keychain: KeychainAccessor())
        _ = Device(device: DeviceAccesor())

        let manager = AppManager()
        self.manager = manager
    }

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { manager.alertState != nil },
            set: { newValue in
                if !newValue {
                    manager.alertState = .none
                }
            }
        )
    }

    var navBarColor: Color {
        switch manager.currentRoute {
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
            let app = manager
            let manager = ImportWalletManager()
            let walletMetadata = try manager.rust.importWallet(enteredWords: [words])
            try app.rust.selectWallet(id: walletMetadata.id)
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                manager.alertState = TaggedItem(.invalidWordGroup)
            case let .WalletAlreadyExists(walletId):
                manager.alertState = TaggedItem(.duplicateWallet(walletId))
            default:
                Log.error("Unable to import wallet: \(error)")
                manager.alertState = TaggedItem(
                    .errorImportingHotWallet(error.localizedDescription))
            }
        } catch {
            Log.error("Unknown error \(error)")
            manager.alertState = TaggedItem(
                .errorImportingHotWallet(error.localizedDescription))
        }
    }

    func importColdWallet(_ export: HardwareExport) {
        let app = manager

        do {
            let wallet = try Wallet.newFromExport(export: export)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            manager.alertState = TaggedItem(.importedSuccessfully)
            try app.rust.selectWallet(id: id)
        } catch let WalletError.WalletAlreadyExists(id) {
            manager.alertState = TaggedItem(.duplicateWallet(id))

            if (try? app.rust.selectWallet(id: id)) == nil {
                manager.alertState = TaggedItem(.unableToSelectWallet)
            }
        } catch {
            manager.alertState = TaggedItem(
                .errorImportingHardwareWallet(error.localizedDescription))
        }
    }

    func handleAddress(_ addressWithNetwork: AddressWithNetwork) {
        let currentNetwork = Database().globalConfig().selectedNetwork()
        let address = addressWithNetwork.address()
        let network = addressWithNetwork.network()
        let selectedWallet = Database().globalConfig().selectedWallet()

        if selectedWallet == nil {
            manager.alertState = TaggedItem(AppAlertState.noWalletSelected(address))
            return
        }

        if network != currentNetwork {
            manager.alertState = TaggedItem(
                AppAlertState.addressWrongNetwork(
                    address: address, network: network, currentNetwork: currentNetwork
                ))
            return
        }

        let amount = addressWithNetwork.amount()
        manager.alertState = TaggedItem(.foundAddress(address, amount))
    }

    func handleTransaction(_ transaction: BitcoinTransaction) {
        Log.debug(
            "Received BitcoinTransaction: \(transaction): \(transaction.txIdHash())"
        )

        let db = Database().unsignedTransactions()
        let txnRecord = db.getTx(txId: transaction.txId())

        guard let txnRecord else {
            manager.alertState = .init(.noUnsignedTransactionFound(transaction.txId()))
            return
        }

        let route = RouteFactory().sendConfirm(
            id: txnRecord.walletId(), details: txnRecord.confirmDetails()
        )

        manager.pushRoute(route)
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
            }
        } catch {
            switch error {
            case let FileHandlerError.NotRecognizedFormat(multiFormatError):
                Log.error("Unrecognized format mulit format error: \(multiFormatError)")
                manager.alertState = TaggedItem(
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
            }
        } catch {
            switch error {
            case let FileHandlerError.NotRecognizedFormat(multiFormatError):
                Log.error("Unrecognized format mulit format error: \(multiFormatError)")
                manager.alertState = TaggedItem(
                    .invalidFileFormat(multiFormatError.localizedDescription))

            default:
                Log.error("Unable to handle scanned code, error: \(error)")
                manager.alertState = TaggedItem(.invalidFileFormat(error.localizedDescription))
            }
        }
    }

    @ViewBuilder
    func SheetContent(_ state: TaggedItem<AppSheetState>) -> some View {
        switch state.item {
        case .qr:
            QrCodeScanView(app: manager, scannedCode: $scannedCode)
        }
    }

    @ViewBuilder
    var BodyView: some View {
        LockView(lockType: manager.authType, isPinCorrect: { pin in AuthPin().check(pin: pin) }, isEnabled: manager.isAuthEnabled) {
            SidebarContainer {
                NavigationStack(path: $manager.router.routes) {
                    RouteView(manager: manager)
                        .navigationDestination(
                            for: Route.self,
                            destination: { route in
                                RouteView(manager: manager, route: route)
                            }
                        )
                        .toolbar {
                            ToolbarItem(placement: .navigationBarLeading) {
                                Button(action: {
                                    withAnimation {
                                        manager.toggleSidebar()
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
        .environment(manager)
    }

    var routeToTint: Color {
        switch manager.router.routes.last {
        case .settings, .walletSettings:
            .blue
        default:
            .white
        }
    }

    func onChangeRoute(_ old: [Route], _ new: [Route]) {
        if !old.isEmpty, new.isEmpty {
            id = UUID()
        }

        manager.dispatch(action: AppAction.updateRoute(routes: new))
    }

    func onChangeQr(
        _: TaggedItem<StringOrData>?, _ scannedCode: TaggedItem<StringOrData>?
    ) {
        guard let scannedCode else { return }
        manager.sheetState = .none
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

    var body: some Scene {
        WindowGroup {
            BodyView
                .implementPopupView()
                .id(id)
                .environment(\.navigate) { route in
                    manager.pushRoute(route)
                }
                .environment(manager)
                .preferredColorScheme(manager.colorScheme)
                .onChange(of: manager.router.routes, onChangeRoute)
                .onChange(of: manager.selectedNetwork) { id = UUID() }
                // QR code scanning
                .onChange(of: scannedCode, onChangeQr)
                // NFC scanning
                .onChange(of: manager.nfcReader.scannedMessage, onChangeNfc)
                .onChange(of: manager.nfcReader.scannedMessageData, onChangeNfcData)
                .alert(
                    manager.alertState?.item.title() ?? "Alert",
                    isPresented: showingAlert,
                    presenting: manager.alertState,
                    actions: alertButtons,
                    message: alertMessage
                )
                .sheet(item: $manager.sheetState, content: SheetContent)
                .gesture(
                    manager.router.routes.isEmpty
                        ? DragGesture()
                        .onChanged { gesture in
                            if gesture.startLocation.x < 25, gesture.translation.width > 100 {
                                withAnimation(.spring()) {
                                    manager.isSidebarVisible = true
                                }
                            }
                        }
                        .onEnded { gesture in
                            if gesture.startLocation.x < 20, gesture.translation.width > 50 {
                                withAnimation(.spring()) {
                                    manager.isSidebarVisible = true
                                }
                            }
                        } : nil
                )
                .task {
                    await manager.rust.initOnStart()
                    await MainActor.run { manager.asyncRuntimeReady = true }
                }
                .onOpenURL(perform: handleFileOpen)
                .onChange(of: phase) { oldPhase, newPhase in
                    Log.debug("[SCENE PHASE]: \(oldPhase) --> \(newPhase)")

                    // TODO: only do this if PIN and/or Biometric is enabledA
                    if newPhase == .background {
                        UIApplication.shared.connectedScenes
                            .compactMap { $0 as? UIWindowScene }
                            .flatMap(\.windows)
                            .forEach { window in
                                window.rootViewController?.dismiss(animated: false)
                            }
                    }
                }
        }
    }
}
