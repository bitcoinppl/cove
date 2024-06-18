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
                NewWalletView(route: NewWalletRoute.select)
                    .navigationDestination(for: Route.self, destination: { route in
                        switch route {
                        case .cove:
                            CoveView(model: model)
                                .onAppear {
                                    print("in main view, router is: \(model.router.routes)")
                                }
                        case .newWallet(route: let route):
                            NewWalletView(route: route)
                        }
                    })
                    .onChange(of: model.router.routes) { _, new in
                        model.dispatch(event: Event.routeChanged(routes: new))
                    }
            }
        }.environment(model)
    }
}
