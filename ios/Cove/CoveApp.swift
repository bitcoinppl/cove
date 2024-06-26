//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

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

    public init() {
        model = MainViewModel()
    }

    var tintColor: Color {
        switch model.router.routes.last {
        case .newWallet(.hotWallet(.select)):
            Color.blue
        case .newWallet:
            Color.white
        default:
            Color.blue
        }
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
                        case let .newWallet(route: route):
                            NewWalletView(route: route)
                        }
                    })
                    .onChange(of: model.router.routes) { _, new in
                        model.dispatch(event: Event.routeChanged(routes: new))
                    }
            }
            .tint(tintColor)
        }
        .environment(model)
        .environment(\.navigate) { route in
            model.pushRoute(route)
        }
    }
}
