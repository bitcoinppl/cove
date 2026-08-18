//
//  TapSignerContainer.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import CoveCore
import SwiftUI

@Observable
class TapSignerManager {
    private let logger = Log(id: "TapSignerManager")

    var id = UUID()
    private var nfc: TapSignerNFC?
    var path: [TapSignerRoute] = []
    var initialRoute: TapSignerRoute

    var enteredPin: String?

    init(_ route: TapSignerRoute) {
        initialRoute = route
    }

    deinit {
        nfc?.cancel()
        enteredPin = nil
        AppManager.shared.tapSignerNfc = nil
    }

    func getOrCreateNfc(_ tapSigner: TapSigner) -> TapSignerNFC {
        if let nfc { return nfc }

        let nfc = TapSignerNFC(tapSigner)
        self.nfc = nfc
        AppManager.shared.tapSignerNfc = nfc

        return self.nfc!
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

        logger.debug(
            "Navigating to \(routeKind(newRoute)), current path count: \(path.count)"
        )
        path.append(newRoute)
    }

    private func routeKind(_ route: TapSignerRoute) -> String {
        switch route {
        case .initSelect: "initSelect"
        case .initAdvanced: "initAdvanced"
        case .startingPin: "startingPin"
        case .newPin: "newPin"
        case .confirmPin: "confirmPin"
        case .setupSuccess: "setupSuccess"
        case .setupRetry: "setupRetry"
        case .importSuccess: "importSuccess"
        case .importRetry: "importRetry"
        case .enterPin: "enterPin"
        }
    }

    func popRoute() {
        if !path.isEmpty { path.removeLast() }
    }

    func cancel() {
        nfc?.cancel()
        enteredPin = nil
        AppManager.shared.tapSignerNfc = nil
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
            TapSignerRouteView(route: manager.initialRoute, manager: manager)
                .navigationDestination(for: TapSignerRoute.self) { route in
                    TapSignerRouteView(route: route, manager: manager)
                }
        }
        .navigationBarTitleDisplayMode(.inline)
        .environment(AuthManager.shared)
        .environment(app)
        .environment(manager)
        .frame(width: screenWidth)
        .id(manager.id)
    }
}

private enum TapSignerSetupRoute {
    case select(TapSigner)
    case advanced(TapSigner)
    case startingPin(TapSigner, String?)
    case newPin(TapSignerNewPinArgs)
    case confirmPin(TapSignerConfirmPinArgs)
    case enterPin(TapSigner, AfterPinAction)
}

private enum TapSignerResultRoute {
    case setupSuccess(TapSigner, TapSignerSetupComplete)
    case setupRetry(TapSigner, SetupCmdResponse)
    case importSuccess(TapSigner, DeriveInfo)
    case importRetry(TapSigner)
}

private enum TapSignerPresentedRoute {
    case setup(TapSignerSetupRoute)
    case result(TapSignerResultRoute)

    init(_ route: TapSignerRoute) {
        switch route {
        case let .initSelect(t):
            self = .setup(.select(t))
        case let .initAdvanced(t):
            self = .setup(.advanced(t))
        case let .startingPin(tapSigner: t, chainCode: chainCode):
            self = .setup(.startingPin(t, chainCode))
        case let .newPin(args: args):
            self = .setup(.newPin(args))
        case let .confirmPin(args):
            self = .setup(.confirmPin(args))
        case let .enterPin(tapSigner: tapSigner, action: action):
            self = .setup(.enterPin(tapSigner, action))
        case let .setupSuccess(tapSigner, setup):
            self = .result(.setupSuccess(tapSigner, setup))
        case let .setupRetry(tapSigner, response):
            self = .result(.setupRetry(tapSigner, response))
        case let .importSuccess(tapSigner, deriveInfo):
            self = .result(.importSuccess(tapSigner, deriveInfo))
        case let .importRetry(tapSigner):
            self = .result(.importRetry(tapSigner))
        }
    }
}

private struct TapSignerRouteView: View {
    let route: TapSignerPresentedRoute
    let manager: TapSignerManager

    init(route: TapSignerRoute, manager: TapSignerManager) {
        self.route = TapSignerPresentedRoute(route)
        self.manager = manager
    }

    var body: some View {
        switch route {
        case let .setup(route):
            TapSignerSetupRouteView(route: route, manager: manager)
        case let .result(route):
            TapSignerResultRouteView(route: route, manager: manager)
        }
    }
}

private struct TapSignerSetupRouteView: View {
    let route: TapSignerSetupRoute
    let manager: TapSignerManager

    var body: some View {
        switch route {
        case let .select(tapSigner):
            TapSignerChooseChainCode(tapSigner: tapSigner)
                .id(id("initSelect"))
        case let .advanced(tapSigner):
            TapSignerAdvancedChainCode(tapSigner: tapSigner)
                .id(id("initAdvanced"))
        case let .startingPin(tapSigner, chainCode):
            TapSignerStartingPin(tapSigner: tapSigner, chainCode: chainCode)
                .id(id("startingPin"))
        case let .newPin(args):
            TapSignerNewPinView(args: args)
                .id(id("newPin"))
        case let .confirmPin(args):
            TapSignerConfirmPinView(args: args)
                .id(id("confirmPin"))
        case let .enterPin(tapSigner, action):
            TapSignerEnterPin(tapSigner: tapSigner, action: action)
                .id(id("enterPin-\(action)"))
        }
    }

    private func id(_ id: String) -> String {
        "\(id)-\(manager.id)"
    }
}

private struct TapSignerResultRouteView: View {
    let route: TapSignerResultRoute
    let manager: TapSignerManager

    var body: some View {
        switch route {
        case let .setupSuccess(tapSigner, setup):
            TapSignerSetupSuccess(tapSigner: tapSigner, setup: setup)
                .id(id("setupSuccess"))
        case let .setupRetry(tapSigner, response):
            TapSignerSetupRetry(tapSigner: tapSigner, response: response)
                .id(id("setupRetry"))
        case let .importSuccess(tapSigner, deriveInfo):
            TapSignerImportSuccess(tapSigner: tapSigner, deriveInfo: deriveInfo)
                .id(id("importSuccess"))
        case let .importRetry(tapSigner):
            TapSignerImportRetry(tapSigner: tapSigner)
        }
    }

    private func id(_ id: String) -> String {
        "\(id)-\(manager.id)"
    }
}

struct TapSignerResultBackground: View {
    var body: some View {
        VStack {
            Image(.chainCodePattern)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .ignoresSafeArea(edges: .all)
                .padding(.top, 5)

            Spacer()
        }
        .opacity(0.8)
    }
}

#Preview {
    TapSignerContainer(route: .initSelect(tapSignerPreviewNew(preview: true)))
}
