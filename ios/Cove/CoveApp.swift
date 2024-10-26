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

    public init() {
        // initialize keychain
        _ = Keychain(keychain: KeychainAccessor())

        let model = MainViewModel()
        self.model = model
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

    func handleFileOpen(_ url: URL) {
        let fileHandler = FileHandler(filePath: url.absoluteString)

        do {
            let readResult = try fileHandler.read()
            switch readResult {
            case .mnemonic(let mnemonic):
                // TODO:
                // create new wallet & send to wallet (check hot wallet import screen)
                ()
            case .hardwareExport(let export):
                // TODO:
                // create new wallet & send to wallet (check hardware wallet import screen)
                ()
            case .address(let addressWithNetwork):
                // TODO:
                // check if network is current network, if not display error
                // if current route is a wallet, pop up a sheet that shows the address, shows mine or external and has link to "Send"
                ()
            }
        }
        catch {
            switch error {
            case FileHandlerError.NotRecognizedFormat(let multiFormatError):
                // TODO: Show this error to the user ignore all other errors?, just log for now
                ()

            case FileHandlerError.OpenFile(let error):
                Log.error("File handler error: \(error)")

            case FileHandlerError.ReadFile(let error):
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
                        .navigationDestination(for: Route.self, destination: { route in
                            RouteView(model: model, route: route)
                        })
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
            .gesture(
                model.router.routes.isEmpty ?
                    DragGesture()
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
