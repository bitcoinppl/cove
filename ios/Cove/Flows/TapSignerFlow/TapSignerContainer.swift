//
//  TapSignerContainer.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerContainer: View {
    let app = AppManager.shared
    let route: TapSignerRoute

    var body: some View {
        Group {
            switch route {
            case let .initSelect(t):
                TapSignerChooseChainCode()
            case .initAdvanced:
                // TapSignerInitAdvanced()
                EmptyView()
            case let .startingPin(t):
                TapSignerStartingPin(tapSigner: t)
            case .newPin:
                // TapSignerNewPin()
                EmptyView()
            case .confirmPin:
                // TapSignerConfirmPin()
                EmptyView()
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .environment(AuthManager.shared)
        .environment(app)
    }
}

#Preview {
    TapSignerContainer(route: .initSelect(tapSignerPreviewNew(preview: true)))
}
