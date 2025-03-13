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
            case .initSelect:
                // TapSignerInitSelect()
                TapSignerStartingPin()
            case .initAdvanced:
                // TapSignerInitAdvanced()
                EmptyView()
            case .startingPin:
                TapSignerStartingPin()
            case .newPin:
                // TapSignerNewPin()
                EmptyView()
            case .confirmPin:
                // TapSignerConfirmPin()
                EmptyView()
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .background(
            ZStack {
                Color(UIColor.systemGroupedBackground)
                    .ignoresSafeArea(edges: .all)

                Image(.settingsPattern)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(maxWidth: .infinity)
                    .ignoresSafeArea(edges: .all)
            }
        )
        .environment(AuthManager.shared)
        .environment(app)
    }
}

#Preview {
    TapSignerContainer(route: .startingPin)
}
