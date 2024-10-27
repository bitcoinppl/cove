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
    @State var model: MainViewModel
    @State var id = UUID()

    // PRIVATE
    @State private var alert: PresentableItem<AlertState>? = .none

    private enum AlertState {
        case invalidWordGroup
        case duplicateWallet(WalletId)
        case errorImportingHotWallet(String)
        case importedSuccessfully
        case unableToSelectWallet
        case errorImportingHardwareWallet(String)
        case invalidFileFormat(String)
        case addressWrongNetwork(address: Address, network: Network, currentNetwork: Network)
        case noWalletSelected(Address)
        case foundAddress(Address)

        func title() -> String {
            switch self {
            case .invalidWordGroup:
                return "Words Not Valid"
            case .duplicateWallet:
                return "Duplicate Wallet"
            case .errorImportingHotWallet:
                return "Error"
            case .importedSuccessfully:
                return "Success"
            case .unableToSelectWallet:
                return "Error"
            case .errorImportingHardwareWallet:
                return "Error Importing Hardware Wallet"
            case .invalidFileFormat:
                return "Invalid File Format"
            case .addressWrongNetwork:
                return "Wrong Network"
            case .noWalletSelected:
                return "No Wallet Selected"
            case .foundAddress:
                return "Found Address"
            }
        }
    }

    @ViewBuilder
    private func alertMessage(alert: PresentableItem<AlertState>) -> some View {
        let text = {
            switch alert.item {
            case .invalidWordGroup:
                return
                    "The words from the file does not create a valid wallet. Please check the words and try again."
            case .duplicateWallet:
                return "This wallet has already been imported!"
            case .errorImportingHotWallet:
                return "Error Importing Wallet"
            case .importedSuccessfully:
                return "Wallet Imported Successfully"
            case .unableToSelectWallet:
                return "Unable to select wallet"
            case .errorImportingHardwareWallet:
                return "Error Importing Hardware Wallet"
            case .invalidFileFormat:
                return "Invalid File Format"
            case .addressWrongNetwork:
                return "Wrong Network"
            case .noWalletSelected:
                return "No Wallet Selected"
            case .foundAddress:
                return "Found Address"
            }
        }()

        Text(text)
    }

    @ViewBuilder
    private func alertButtons(alert: PresentableItem<AlertState>) -> some View {
        switch alert.item {
        case let .duplicateWallet(walletId):
            Button("OK") {
                self.alert = .none
                try? model.rust.selectWallet(id: walletId)
                model.resetRoute(to: .selectedWallet(walletId))
            }
        case .invalidWordGroup,
             .errorImportingHotWallet,
             .importedSuccessfully,
             .unableToSelectWallet,
             .errorImportingHardwareWallet,
             .invalidFileFormat:
            Button("OK") {
                self.alert = .none
            }
        case let .addressWrongNetwork(address: address, network: _, currentNetwork: _):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                self.alert = .none
            }
        case let .noWalletSelected(address):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }
            Button("Cancel") {
                self.alert = .none
            }
        case let .foundAddress(address):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Send To Address") {
                // TODO: Route to Send for the wallet
                self.alert = .none
            }
            Button("Cancel") {
                self.alert = .none
            }
        }
    }

    public init() {
        // initialize keychain
        _ = Keychain(keychain: KeychainAccessor())

        let model = MainViewModel()
        self.model = model
    }

    var showingAlert: Binding<Bool> {
        Binding(
            get: { self.alert != nil },
            set: { newValue in
                if !newValue {
                    self.alert = .none
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

    func importHotWallet(_ words: [String]) {
        do {
            let app = model
            let model = ImportWalletViewModel()
            let walletMetadata = try model.rust.importWallet(enteredWords: [words])
            try app.rust.selectWallet(id: walletMetadata.id)
            app.resetRoute(to: .selectedWallet(walletMetadata.id))
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                self.alert = PresentableItem(.invalidWordGroup)
            case let .WalletAlreadyExists(walletId):
                self.alert = PresentableItem(.duplicateWallet(walletId))
            default:
                Log.error("Unable to import wallet: \(error)")
                self.alert = PresentableItem(
                    .errorImportingHotWallet(error.localizedDescription))
            }
        } catch {
            Log.error("Unknown error \(error)")
            alert = PresentableItem(
                .errorImportingHotWallet(error.localizedDescription))
        }
    }

    func importColdWallet(_ export: HardwareExport) {
        let app = model

        do {
            let wallet = try Wallet.newFromExport(export: export)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            alert = PresentableItem(.importedSuccessfully)
            try app.rust.selectWallet(id: id)
        } catch let WalletError.WalletAlreadyExists(id) {
            self.alert = PresentableItem(.duplicateWallet(id))

            if (try? app.rust.selectWallet(id: id)) == nil {
                self.alert = PresentableItem(.unableToSelectWallet)
            }
        } catch {
            alert = PresentableItem(.errorImportingHardwareWallet(error.localizedDescription))
        }
    }

    func handleAddress(_ addressWithNetwork: AddressWithNetwork) {
        let currentNetwork = Database().globalConfig().selectedNetwork()
        let address = addressWithNetwork.address()
        let network = addressWithNetwork.network()
        let selectedWallet = Database().globalConfig().selectedWallet()

        if selectedWallet == nil {
            return alert = PresentableItem(AlertState.noWalletSelected(address))
        }

        if network != currentNetwork {
            return alert = PresentableItem(AlertState.addressWrongNetwork(address: address, network: network, currentNetwork: currentNetwork))
        }

        return alert = PresentableItem(.foundAddress(address))
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
                alert = PresentableItem(.invalidFileFormat(multiFormatError.localizedDescription))

            case let FileHandlerError.OpenFile(error):
                Log.error("File handler error: \(error)")

            case let FileHandlerError.ReadFile(error):
                Log.error("Unable to read file: \(error)")

            case FileHandlerError.FileNotFound:
                Log.error("File not found")

            default:
                ()
            }
        }
    }

    var body: some Scene {
        WindowGroup {
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

                SidebarView(isShowing: $model.isSidebarVisible, currentRoute: model.currentRoute)
            }
            .implementPopupView()
            .id(id)
            .environment(\.navigate) { route in
                model.pushRoute(route)
            }
            .environment(model)
            .preferredColorScheme(model.colorScheme)
            .onChange(of: model.router.routes) { old, new in
                if !old.isEmpty && new.isEmpty {
                    id = UUID()
                }

                model.dispatch(action: AppAction.updateRoute(routes: new))
            }
            .onChange(of: model.selectedNetwork) {
                id = UUID()
            }
            .alert(
                alert?.item.title() ?? "Alert",
                isPresented: showingAlert,
                presenting: alert,
                actions: alertButtons,
                message: alertMessage
            )
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
