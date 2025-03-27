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

    var id = UUID()
    var nfc: TapSignerNFC?
    var path: [TapSignerRoute] = []
    var initialRoute: TapSignerRoute

    var enteredPin: String?

    init(_ route: TapSignerRoute) {
        initialRoute = route
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

    func resetRoute(to route: TapSignerRoute) {
        path = []
        initialRoute = route
        id = UUID()
    }
}

struct TapSignerContainer: View {
    let app = AppManager.shared
    @State var manager: TapSignerManager

    init(route: TapSignerRoute) {
        manager = TapSignerManager(route)
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
        .id(manager.id)
    }

    @ViewBuilder
    func routeContent(route: TapSignerRoute) -> some View {
        switch route {
        case let .initSelect(t):
            TapSignerChooseChainCode(tapSigner: t)
                .id(id("initSelect"))
        case let .initAdvanced(t):
            TapSignerAdvancedChainCode(tapSigner: t)
                .id(id("initAdvanced"))
        case let .startingPin(tapSigner: t, chainCode: chainCode):
            TapSignerStartingPin(tapSigner: t, chainCode: chainCode)
                .id(id("startingPin"))
        case let .newPin(tapSigner: t, startingPin: pin, chainCode: chainCode):
            TapSignerNewPin(tapSigner: t, startingPin: pin, chainCode: chainCode)
                .id(id("newPin"))
        case let .confirmPin(
            tapSigner: t, startingPin: startingPin, newPin: newPin, chainCode: chainCode
        ):
            TapSignerConfirmPin(
                tapSigner: t, startingPin: startingPin, newPin: newPin, chainCode: chainCode
            )
            .id(id("confirmPin"))
        case let .setupSuccess(tapSigner, setup):
            TapSignerSetupSuccess(tapSigner: tapSigner, setup: setup)
                .id(id("setupSuccess"))
        case let .setupRetry(tapSigner, response):
            TapSignerSetupRetry(tapSigner: tapSigner, response: response)
                .id(id("setupRetry"))
        case let .enterPin(tapSigner: t, action: action):
            TapSignerEnterPin(tapSigner: t, action: action)
                .id(id("enterPin-\(action)"))
        case let .importSuccess(t, deriveInfo):
            TapSignerImportSuccess(tapSigner: t, deriveInfo: deriveInfo)
                .id(id("importSuccess"))
        case let .importRetry(t):
            TapSignerImportRetry(tapSigner: t)
        }
    }

    private func id(_ id: String) -> String {
        "\(id)-\(manager.id)"
    }
}

#Preview {
    TapSignerContainer(route: .initSelect(tapSignerPreviewNew(preview: true)))
}
