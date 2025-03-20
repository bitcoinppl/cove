//
//  TapSignerContainer.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

@Observable
class TapSignerManager {
    private let logger = Log(id: "TapSignerManager")

    var path: [TapSignerRoute] = []
    var initialRoute: TapSignerRoute

    init(_ route: TapSignerRoute) {
        self.initialRoute = route
    }

    func navigate(to newRoute: TapSignerRoute) {
        // don't allow navigating to the same route
        if let lastRoute = path.last {
            switch (lastRoute, newRoute) {
            case (.initSelect, .initSelect),
                 (.initAdvanced, .initAdvanced),
                 (.startingPin, .startingPin),
                 (.newPin, .newPin),
                 (.confirmPin, .confirmPin):
                return
            default: ()
            }
        }

        logger.debug("Navigating to \(newRoute), current path: \(path)")
        path.append(newRoute)
    }

    func popRoute() {
        if !path.isEmpty { path.removeLast() }
    }
}

struct TapSignerContainer: View {
    let app = AppManager.shared
    @State var manager: TapSignerManager

    init(route: TapSignerRoute) {
        self.manager = TapSignerManager(route)
    }

    var body: some View {
        NavigationStack(path: $manager.path) {
            // Initial view based on initial route
            routeContent(route: manager.initialRoute)
                .navigationDestination(for: TapSignerRoute.self) { route in
                    routeContent(route: route)
                }
        }
        .navigationBarTitleDisplayMode(.inline)
        .environment(AuthManager.shared)
        .environment(app)
        .environment(manager)
        .frame(width: screenWidth)
    }

    @ViewBuilder
    func routeContent(route: TapSignerRoute) -> some View {
        switch route {
        case let .initSelect(t):
            TapSignerChooseChainCode(tapSigner: t)
                .id("initSelect")
        case .initAdvanced:
            EmptyView()
                .id("initAdvanced")
        case let .startingPin(t):
            TapSignerStartingPin(tapSigner: t)
                .id("startingPin")
        case let .newPin(tapSigner: t, startingPin: pin):
            TapSignerNewPin(tapSigner: t, startingPin: pin)
                .id("newPin")
        case let .confirmPin(tapSigner: t, startingPin: startingPin, newPin: newPin):
            TapSignerConfirmPin(tapSigner: t, startingPin: startingPin, newPin: newPin)
                .id("confirmPin")
        }
    }
}

#Preview {
    TapSignerContainer(route: .initSelect(tapSignerPreviewNew(preview: true)))
}
