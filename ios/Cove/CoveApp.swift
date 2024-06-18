//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

import SwiftUI

@main
struct CoveApp: App {
    @State var model: MainViewModel

    public init() {
        self.model = MainViewModel()
    }

    var body: some Scene {
        WindowGroup {
            NavigationStack(path: $model.router.routes) {
                VStack(spacing: 20) {
                    Button(action: { model.pushRoute(RouteFactory().newWalletDefault()) }) {
                        Text("Push Route NewWallet")
                    }

                    Button(action: { model.pushRoute(Route.cove) }) {
                        Text("Push Route Cove")
                    }

                    Button(action: { model.setRoute([RouteFactory().newWalletDefault()]) }) {
                        Text("Set Route")
                    }

                    Button(action: { try! model.database.toggleBoolConfig(key: GlobalBoolConfigKey.completedOnboarding) }) {
                        Text("Onboarding: \(try! model.database.getBoolConfig(key: GlobalBoolConfigKey.completedOnboarding))")
                    }
                }
                .navigationDestination(for: Route.self, destination: { route in
                    switch route {
                    case .cove:
                        CoveView(model: model)
                            .onAppear {
                                print("in main view, router is: \(model.router.routes)")
                            }
                    case .newWallet(route: let new_wallet_route):
                        NewWalletView()
                            .onAppear {
                                print("in wallet, router is: \(model.router.routes)")
                            }
                    }
                })
                .onChange(of: model.router.routes) { _, new in
                    model.dispatch(event: Event.routeChanged(routes: new))
                }
            }
        }
    }
}
