//
//  TapSignerContainer.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

import SwiftUI

@Observable
class TapSignerManager {
    var route = NavigationPath()
    var initialRoute: TapSignerRoute

    init(_ route: TapSignerRoute) {
        self.initialRoute = route
    }

    func navigate(to newRoute: TapSignerRoute) {
        route.append(newRoute)
    }

    func popRoute() {
        if !route.isEmpty { route.removeLast() }
    }
}

struct TapSignerContainer: View {
    let app = AppManager.shared
    @State var manager: TapSignerManager

    init(route: TapSignerRoute) {
        self.manager = TapSignerManager(route)
    }

    var body: some View {
        NavigationStack(path: $manager.route) {
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
        case .confirmPin:
            EmptyView()
                .id("confirmPin")
        }
    }
}

#Preview {
    TapSignerContainer(route: .initSelect(tapSignerPreviewNew(preview: true)))
}
