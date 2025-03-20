//
//  TapSignerContainer.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

@Observable
class TapSignerManager {
    var route: TapSignerRoute

    init(_ route: TapSignerRoute) {
        self.route = route
    }
}

struct TapSignerContainer: View {
    let app = AppManager.shared
    let manager: TapSignerManager

    init(route: TapSignerRoute) {
        self.manager = TapSignerManager(route)
    }

    var body: some View {
        ZStack {
            switch manager.route {
            case let .initSelect(t):
                TapSignerChooseChainCode(tapSigner: t)
                    .id("initSelect")
                    .transition(.navigationTransitionNext)
                    .zIndex(1)
            case .initAdvanced:
                // TapSignerInitAdvanced()
                EmptyView()
                    .id("initAdvanced")
                    .transition(.navigationTransitionNext)
                    .zIndex(1)
            case let .startingPin(t):
                TapSignerStartingPin(tapSigner: t)
                    .id("startingPin")
                    .transition(.navigationTransitionNext)
                    .zIndex(1)
            case let .newPin(tapSigner: t, startingPin: pin):
                TapSignerNewPin(tapSigner: t, startingPin: pin)
                    .id("newPin")
                    .transition(.navigationTransitionNext)
                    .zIndex(1)
            case .confirmPin:
                // TapSignerConfirmPin()
                EmptyView()
                    .id("confirmPin")
                    .transition(.navigationTransitionNext)
                    .zIndex(1)
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .environment(AuthManager.shared)
        .environment(app)
        .environment(manager)
    }
}

#Preview {
    TapSignerContainer(route: .initSelect(tapSignerPreviewNew(preview: true)))
}
