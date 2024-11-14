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

@main
struct CoveApp: App {
    @Environment(\.colorScheme) var colorScheme

    @State var model: MainViewModel
    @State var id = UUID()

    @State var scannedCode: TaggedItem<StringOrData>? = .none

    @ViewBuilder
    private func alertMessage(alert: TaggedItem<AppAlertState>) -> some View {
        let text = {
            switch alert.item {
            case .invalidWordGroup:
                return
                    "The words from the file does not create a valid wallet. Please check the words and try again."
            case .duplicateWallet:
                return "This wallet has already been imported! Taking you there now..."
            case .errorImportingHotWallet:
                return "Error Importing Wallet"
            case .importedSuccessfully:
                return "Wallet Imported Successfully"
            case .unableToSelectWallet:
                return "Unable to select wallet, please try again"
            case let .errorImportingHardwareWallet(error):
                return "Error: \(error)"
            case .invalidFileFormat:
                return "The file or scanned code did not match any formats that Cove supports."
            case let .addressWrongNetwork(
                address: address, network: network, currentNetwork: currentNetwork
            ):
                return
                    "The address \(address) is on the wrong network. You are on \(currentNetwork), and the address was for \(network)."
            case let .noWalletSelected(address),
                 let .foundAddress(address, _):
                return String(address)
            case .noCameraPermission:
                return "Please allow camera access in Settings to use this feature."
            case let .failedToScanQr(error):
                return "Error: \(error)"
            }
        }()

        Text(text)
    }

    @ViewBuilder
    private func alertButtons(alert: TaggedItem<AppAlertState>) -> some View {
        switch alert.item {
        case let .duplicateWallet(walletId):
            Button("OK") {
                model.alertState = .none
                try? model.rust.selectWallet(id: walletId)
            }
        case .invalidWordGroup,
             .errorImportingHotWallet,
             .importedSuccessfully,
             .unableToSelectWallet,
             .errorImportingHardwareWallet,
             .invalidFileFormat:
            Button("OK") {
                model.alertState = .none
            }
        case let .addressWrongNetwork(address: address, network: _, currentNetwork: _):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                model.alertState = .none
            }
        case let .noWalletSelected(address):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                model.alertState = .none
            }
        case let .foundAddress(address, amount):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            if let id = Database().globalConfig().selectedWallet() {
                Button("Send To Address") {
                    let route = RouteFactory().sendSetAmount(id: id, address: address, amount: amount)
                    model.pushRoute(route)
                    model.alertState = .none
                }
            }

            Button("Cancel") {
                model.alertState = .none
            }
        case .noCameraPermission:
            Button("OK") {
                model.alertState = .none
                let url = URL(string: UIApplication.openSettingsURLString)!
                UIApplication.shared.open(url)
            }
        case .failedToScanQr:
            Button("OK") {
                model.alertState = .none
            }
        }
    }

    public init() {
        // initialize keychain
        _ = Keychain(keychain: KeychainAccessor())

        let model = MainViewModel()
        self.model = model
    }

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { model.alertState != nil },
            set: { newValue in
                if !newValue {
                    model.alertState = .none
                }
            }
        )
    }

    var navBarColor: Color {
        switch model.currentRoute {
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
            let app = model
            let model = ImportWalletViewModel()
            let walletMetadata = try model.rust.importWallet(enteredWords: [words])
            try app.rust.selectWallet(id: walletMetadata.id)
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                model.alertState = TaggedItem(.invalidWordGroup)
            case let .WalletAlreadyExists(walletId):
                model.alertState = TaggedItem(.duplicateWallet(walletId))
            default:
                Log.error("Unable to import wallet: \(error)")
                model.alertState = TaggedItem(
                    .errorImportingHotWallet(error.localizedDescription))
            }
        } catch {
            Log.error("Unknown error \(error)")
            model.alertState = TaggedItem(
                .errorImportingHotWallet(error.localizedDescription))
        }
    }

    func importColdWallet(_ export: HardwareExport) {
        let app = model

        do {
            let wallet = try Wallet.newFromExport(export: export)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            model.alertState = TaggedItem(.importedSuccessfully)
            try app.rust.selectWallet(id: id)
        } catch let WalletError.WalletAlreadyExists(id) {
            model.alertState = TaggedItem(.duplicateWallet(id))

            if (try? app.rust.selectWallet(id: id)) == nil {
                model.alertState = TaggedItem(.unableToSelectWallet)
            }
        } catch {
            model.alertState = TaggedItem(.errorImportingHardwareWallet(error.localizedDescription))
        }
    }

    func handleAddress(_ addressWithNetwork: AddressWithNetwork) {
        let currentNetwork = Database().globalConfig().selectedNetwork()
        let address = addressWithNetwork.address()
        let network = addressWithNetwork.network()
        let selectedWallet = Database().globalConfig().selectedWallet()

        if selectedWallet == nil {
            model.alertState = TaggedItem(AppAlertState.noWalletSelected(address))
            return
        }

        if network != currentNetwork {
            model.alertState = TaggedItem(
                AppAlertState.addressWrongNetwork(
                    address: address, network: network, currentNetwork: currentNetwork
                ))
            return
        }

        let amount = addressWithNetwork.amount()
        model.alertState = TaggedItem(.foundAddress(address, amount))
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
            }
        } catch {
            switch error {
            case let FileHandlerError.NotRecognizedFormat(multiFormatError):
                Log.error("Unrecognized format mulit format error: \(multiFormatError)")
                model.alertState = TaggedItem(
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
            print("MULTI FORMAT: \(multiFormat)")
            switch multiFormat {
            case let .mnemonic(mnemonic):
                importHotWallet(mnemonic.words())
            case let .hardwareExport(export):
                importColdWallet(export)
            case let .address(addressWithNetwork):
                handleAddress(addressWithNetwork)
            }
        } catch {
            switch error {
            case let FileHandlerError.NotRecognizedFormat(multiFormatError):
                Log.error("Unrecognized format mulit format error: \(multiFormatError)")
                model.alertState = TaggedItem(
                    .invalidFileFormat(multiFormatError.localizedDescription))

            default:
                Log.error("Unable to handle scanned code, error: \(error)")
                model.alertState = TaggedItem(.invalidFileFormat(error.localizedDescription))
            }
        }
    }

    @ViewBuilder
    func SheetContent(_ state: TaggedItem<AppSheetState>) -> some View {
        switch state.item {
        case .qr:
            QrCodeScanView(app: model, scannedCode: $scannedCode)
        }
    }

    @ViewBuilder
    var BodyView: some View {
        ZStack {
            NavigationStack(path: $model.router.routes) {
                RouteView(model: model)
                    .navigationDestination(
                        for: Route.self,
                        destination: { route in
                            RouteView(model: model, route: route)
                        }
                    )
                    .toolbar {
                        ToolbarItem(placement: .navigationBarLeading) {
                            Button(action: {
                                withAnimation {
                                    model.toggleSidebar()
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

            SidebarView(isShowing: $model.isSidebarVisible, currentRoute: model.currentRoute)
        }
    }

    var routeToTint: Color {
        switch model.router.routes.last {
        case .send:
            .white
        case .selectedWallet:
            .white
        default:
            .blue
        }
    }

    func onChangeRoute(_ old: [Route], _ new: [Route]) {
        if !old.isEmpty && new.isEmpty {
            id = UUID()
        }

        model.dispatch(action: AppAction.updateRoute(routes: new))
    }

    func onChangeQr(
        _: TaggedItem<StringOrData>?, _ scannedCode: TaggedItem<StringOrData>?
    ) {
        guard let scannedCode else { return }
        model.sheetState = .none
        handleScannedCode(scannedCode.item)
    }

    func onChangeNfc(_: String?, _ scannedMessage: String?) {
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
                    model.pushRoute(route)
                }
                .environment(model)
                .preferredColorScheme(model.colorScheme)
                .onChange(of: model.router.routes, onChangeRoute)
                .onChange(of: model.selectedNetwork) { id = UUID() }
                // QR code scanning
                .onChange(of: scannedCode, onChangeQr)
                // NFC scanning
                .onChange(of: model.nfcReader.scannedMessage, onChangeNfc)
                .alert(
                    model.alertState?.item.title() ?? "Alert",
                    isPresented: showingAlert,
                    presenting: model.alertState,
                    actions: alertButtons,
                    message: alertMessage
                )
                .sheet(item: $model.sheetState, content: SheetContent)
                .gesture(
                    model.router.routes.isEmpty
                        ? DragGesture()
                        .onChanged { gesture in
                            if gesture.startLocation.x < 25, gesture.translation.width > 100 {
                                withAnimation(.spring()) {
                                    model.isSidebarVisible = true
                                }
                            }
                        }
                        .onEnded { gesture in
                            if gesture.startLocation.x < 20, gesture.translation.width > 50 {
                                withAnimation(.spring()) {
                                    model.isSidebarVisible = true
                                }
                            }
                        } : nil
                )
                .task {
                    await model.rust.initOnStart()
                    await MainActor.run {
                        model.asyncRuntimeReady = true
                    }
                }
                .onOpenURL(perform: handleFileOpen)
        }
    }
}
