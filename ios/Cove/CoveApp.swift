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
    @State private var alertState: PresentableItem<AlertState>? = .none

    private enum AlertState {
        case invalidWordGroup
        case duplicateWallet(WalletId)
        case errorImportingHotWallet(String)
    }

    public init() {
        // initialize keychain
        _ = Keychain(keychain: KeychainAccessor())

        let model = MainViewModel()
        self.model = model
    }

    private func alertFrom(_ state: PresentableItem<AlertState>) -> Alert {
        switch state.item {
        case .invalidWordGroup:
            return Alert(
                title: Text("Words Not Valid"),
                message: Text(
                    "The words from the file does not create a valid wallet. Please check the words and try again."
                ),
                dismissButton: .default(Text("OK")) {
                    self.alertState = .none
                }
            )

        case let .duplicateWallet(walletId):
            return Alert(
                title: Text("Duplicate Wallet"),
                message: Text("This wallet has already been imported!"),
                dismissButton: .default(Text("OK")) {
                    self.alertState = .none
                    try? self.model.rust.selectWallet(id: walletId)
                    self.model.resetRoute(to: .selectedWallet(walletId))
                }
            )

        case let .errorImportingHotWallet(error):
            return Alert(
                title: Text("Error Importing Wallet"),
                message: Text(error),
                dismissButton: .default(Text("OK")) {
                    self.alertState = .none
                }
            )
        }
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
            let app = self.model
            let model = ImportWalletViewModel()
            let walletMetadata = try model.rust.importWallet(enteredWords: [words])
            try app.rust.selectWallet(id: walletMetadata.id)
            app.resetRoute(to: .selectedWallet(walletMetadata.id))
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                self.alertState = PresentableItem(.invalidWordGroup)
            case let .WalletAlreadyExists(walletId):
                self.alertState = PresentableItem(.duplicateWallet(walletId))
            default:
                Log.error("Unable to import wallet: \(error)")
                self.alertState = PresentableItem(
                    .errorImportingHotWallet(error.localizedDescription))
            }
        } catch {
            Log.error("Unknown error \(error)")
            alertState = PresentableItem(
                .errorImportingHotWallet(error.localizedDescription))
        }
    }

    func handleFileOpen(_ url: URL) {
        let fileHandler = FileHandler(filePath: url.absoluteString)

        do {
            let readResult = try fileHandler.read()
            switch readResult {
            case let .mnemonic(mnemonic):
                importHotWallet(mnemonic.words())
            case let .hardwareExport(export):
                // TODO:
                // create new wallet & send to wallet (check hardware wallet import screen)
                ()
            case let .address(addressWithNetwork):
                // TODO:
                // check if network is current network, if not display error
                // if current route is a wallet, pop up a sheet that shows the address, shows mine or external and has link to "Send"
                ()
            }
        } catch {
            switch error {
            case let FileHandlerError.NotRecognizedFormat(multiFormatError):
                // TODO: Show this error to the user ignore all other errors?, just log for now
                ()

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
            .alert(item: $alertState, content: alertFrom)
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
